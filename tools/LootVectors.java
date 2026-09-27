// Differential test vectors for kiln-loot: vanilla 26.3's own loot tables evaluated in-process
// (datapack loaded like the dedicated server loads it) with controlled contexts and seeds.
//
// usage: java -cp <server jar + libraries> tools/LootVectors.java <out-dir> <contexts per table> <seed>
// (tools/loot_vectors.py builds the classpath)
//
// Writes <out-dir>/<kind>.jsonl, one case per line: the table, the random mode (a random sequence
// from a world seed, an explicit loot seed, or LootTable.fill into a container), the context
// (tool, block state, explosion radius, entities and their enchantment levels, block entity
// components, luck), vanilla's answers for every entity / damage source / location predicate
// the datapack uses (so kiln's context hook can answer the same), and the resulting stacks.
//
// The server and level are allocated without running their constructors; only what loot
// evaluation touches is provided (random sequences, registries, a tiny block map, one biome).

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.google.gson.JsonPrimitive;
import com.mojang.serialization.JsonOps;
import com.mojang.serialization.Lifecycle;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.TreeMap;
import java.util.stream.Stream;
import net.minecraft.advancements.predicates.DamageSourcePredicate;
import net.minecraft.advancements.predicates.LocationPredicate;
import net.minecraft.advancements.predicates.entity.EntityPredicate;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderSet;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.component.TypedDataComponent;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.ReloadableServerRegistries;
import net.minecraft.server.ReloadableServerResources;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.util.RandomSource;
import net.minecraft.util.Util;
import net.minecraft.util.context.ContextKeySet;
import net.minecraft.world.Nameable;
import net.minecraft.world.RandomSequences;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.damagesource.DamageSources;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.animal.sheep.Sheep;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.crafting.RecipeManager;
import net.minecraft.world.item.enchantment.Enchantment;
import net.minecraft.world.item.enchantment.EnchantmentHelper;
import net.minecraft.world.item.enchantment.Enchantments;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.StructureStart;
import net.minecraft.world.level.storage.loot.LootParams;
import net.minecraft.world.level.storage.loot.LootTable;
import net.minecraft.world.level.storage.loot.parameters.LootContextParamSets;
import net.minecraft.world.level.storage.loot.parameters.LootContextParams;
import net.minecraft.world.phys.Vec3;
import sun.misc.Unsafe;

public class LootVectors {
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);
    static RegistryAccess access;
    static ReloadableServerResources resources;
    static RecipeManager recipes;
    static Unsafe U;
    static FakeLevel LEVEL;
    static FakeServer SERVER;
    static DamageSources DAMAGE;
    static RandomSequences SEQS = new RandomSequences();
    static long WORLD_SEED;
    static final Map<BlockPos, BlockState> WORLD = new HashMap<>();
    static Holder<Biome> BIOME;
    static Path DATAPACK;

    record Loaded(ReloadableServerResources resources, RegistryAccess.Frozen access) {}

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Path out = Path.of(args[0]);
        int contexts = Integer.parseInt(args[1]);
        long seed = Long.parseLong(args[2]);
        DATAPACK = Path.of(args.length > 3 ? args[3] : "generated");
        load();
        setup();
        collectPredicates();
        Files.createDirectories(out);
        if (args.length > 4) synthetic(Path.of(args[4]), out, contexts, seed);
        else run(out, contexts, seed);
    }

    // ---- loading and fakes ----------------------------------------------------------------------

    static void load() throws Exception {
        PackRepository repo = ServerPacksSource.createVanillaTrustedRepository();
        var packs = new WorldLoader.PackConfig(repo, WorldDataConfiguration.DEFAULT, false, true);
        var init = new WorldLoader.InitConfig(packs, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
        Loaded loaded = WorldLoader.<Void, Loaded>load(init, ctx -> {
            Registry<LevelStem> none = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
            var dims = ctx.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                    .getOrThrow(WorldPresets.NORMAL).value().createWorldDimensions().bake(none);
            return new WorldLoader.DataLoadOutput<>(null, dims.dimensionsRegistryAccess());
        }, (manager, res, layers, cookie) -> {
            manager.close();
            return new Loaded(res, layers.compositeAccess());
        }, Util.backgroundExecutor(), Runnable::run).get();
        loaded.resources().updateComponentsAndStaticRegistryTags();
        access = loaded.access();
        resources = loaded.resources();
        recipes = resources.getRecipeManager();
        recipes.finalizeRecipeLoading(FeatureFlags.DEFAULT_FLAGS);
    }

    static void set(Object target, Class<?> owner, String field, Object value) throws Exception {
        Field f = owner.getDeclaredField(field);
        f.setAccessible(true);
        f.set(target, value);
    }

    static void setup() throws Exception {
        Field uf = Unsafe.class.getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        U = (Unsafe) uf.get(null);
        SERVER = (FakeServer) U.allocateInstance(FakeServer.class);
        LEVEL = (FakeLevel) U.allocateInstance(FakeLevel.class);
        set(LEVEL, Level.class, "registryAccess", access);
        set(LEVEL, Level.class, "random", RandomSource.create(0));
        DAMAGE = new DamageSources(access);
        BIOME = access.lookupOrThrow(Registries.BIOME).getOrThrow(Biomes.PLAINS);
        STRUCTURES = (FakeStructureManager) U.allocateInstance(FakeStructureManager.class);
    }

    static FakeStructureManager STRUCTURES;

    static class FakeServer extends DedicatedServer {
        FakeServer() {
            super(null, null, null, null, null, null, null, null, null, null);
        }

        @Override
        public RandomSource getRandomSequence(Identifier id) {
            return SEQS.get(id, WORLD_SEED);
        }

        @Override
        public ReloadableServerRegistries.Holder reloadableRegistries() {
            return resources.fullRegistries();
        }
    }

    static class FakeStructureManager extends StructureManager {
        FakeStructureManager() {
            super(null, null, null);
        }

        @Override
        public StructureStart getStructureAt(BlockPos pos, HolderSet<Structure> structures) {
            return StructureStart.INVALID_START;
        }
    }

    static class FakeLevel extends ServerLevel {
        FakeLevel() {
            super(null, null, null, null, null, null, false, 0L, null, false);
        }

        @Override
        public MinecraftServer getServer() {
            return SERVER;
        }

        @Override
        public RecipeManager recipeAccess() {
            return recipes;
        }

        @Override
        public FeatureFlagSet enabledFeatures() {
            return FeatureFlags.DEFAULT_FLAGS;
        }

        @Override
        public DamageSources damageSources() {
            return DAMAGE;
        }

        @Override
        public boolean isRaining() {
            return false;
        }

        @Override
        public boolean isThundering() {
            return false;
        }

        @Override
        public boolean isLoaded(BlockPos pos) {
            return true;
        }

        @Override
        public BlockState getBlockState(BlockPos pos) {
            return WORLD.getOrDefault(pos, Blocks.AIR.defaultBlockState());
        }

        @Override
        public BlockEntity getBlockEntity(BlockPos pos) {
            return null;
        }

        @Override
        public Holder<Biome> getBiome(BlockPos pos) {
            return BIOME;
        }

        @Override
        public ResourceKey<Level> dimension() {
            return Level.OVERWORLD;
        }

        @Override
        public StructureManager structureManager() {
            return STRUCTURES;
        }

        @Override
        public BlockPos findNearestMapStructure(HolderSet<Structure> structures, BlockPos pos, int radius, boolean skip) {
            return null;
        }

        @Override
        public long getGameTime() {
            return 0;
        }

        int nextEntityId;

        @Override
        public net.minecraft.world.Difficulty getDifficulty() {
            return net.minecraft.world.Difficulty.NORMAL;
        }

        @Override
        public int getNextEntityId() {
            return ++nextEntityId;
        }
    }

    static class FakePlayer extends ServerPlayer {
        FakePlayer() {
            super(null, null, null, null);
        }
    }

    // ---- encoding helpers ------------------------------------------------------------------------

    static String hex(ItemStack s) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, s);
        return ByteBufUtil.hexDump(buf);
    }

    static String hex(TypedDataComponent<?> c) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        TypedDataComponent.STREAM_CODEC.encode(buf, c);
        return ByteBufUtil.hexDump(buf);
    }

    static String hexText(Component c) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        ComponentSerialization.STREAM_CODEC.encode(buf, c);
        return ByteBufUtil.hexDump(buf);
    }

    static String str(String s) {
        StringBuilder b = new StringBuilder("\"");
        for (char c : s.toCharArray()) {
            switch (c) {
                case '"' -> b.append("\\\"");
                case '\\' -> b.append("\\\\");
                case '\n' -> b.append("\\n");
                case '\r' -> b.append("\\r");
                case '\t' -> b.append("\\t");
                default -> {
                    if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
                    else b.append(c);
                }
            }
        }
        return b.append('"').toString();
    }

    /** Compact JSON with sorted keys and numbers as written (kiln-loot's Json::canonical). */
    static String canonical(JsonElement e) {
        if (e == null || e.isJsonNull()) return "null";
        if (e.isJsonPrimitive()) {
            JsonPrimitive p = e.getAsJsonPrimitive();
            if (p.isBoolean()) return p.getAsBoolean() ? "true" : "false";
            if (p.isNumber()) return p.getAsString();
            return str(p.getAsString());
        }
        if (e.isJsonArray()) {
            List<String> parts = new ArrayList<>();
            for (JsonElement x : e.getAsJsonArray()) parts.add(canonical(x));
            return "[" + String.join(",", parts) + "]";
        }
        TreeMap<String, JsonElement> sorted = new TreeMap<>();
        for (var entry : e.getAsJsonObject().entrySet()) sorted.put(entry.getKey(), entry.getValue());
        List<String> parts = new ArrayList<>();
        for (var entry : sorted.entrySet()) parts.add(str(entry.getKey()) + ":" + canonical(entry.getValue()));
        return "{" + String.join(",", parts) + "}";
    }

    // ---- predicates the datapack uses ------------------------------------------------------------

    record EntityPred(String target, String key, EntityPredicate predicate) {}
    record DamagePred(String key, DamageSourcePredicate predicate) {}
    record LocationPred(String key, int[] offset, LocationPredicate predicate) {}

    static final Map<String, EntityPred> ENTITY_PREDS = new LinkedHashMap<>();
    static final Map<String, DamagePred> DAMAGE_PREDS = new LinkedHashMap<>();
    static final Map<String, LocationPred> LOCATION_PREDS = new LinkedHashMap<>();

    static void collectPredicates() throws Exception {
        var ops = access.createSerializationContext(JsonOps.INSTANCE);
        Path root = DATAPACK.resolve("data/minecraft");
        for (String dir : List.of("loot_table", "predicate", "item_modifier", "context_int_provider", "context_float_provider")) {
            Path d = root.resolve(dir);
            if (!Files.isDirectory(d)) continue;
            try (Stream<Path> files = Files.walk(d)) {
                for (Path f : files.filter(p -> p.toString().endsWith(".json")).toList()) {
                    walk(JsonParser.parseString(Files.readString(f)), ops);
                }
            }
        }
        OUT.println("predicates: " + ENTITY_PREDS.size() + " entity, " + DAMAGE_PREDS.size() + " damage, " + LOCATION_PREDS.size() + " location");
    }

    static void walk(JsonElement e, com.mojang.serialization.DynamicOps<JsonElement> ops) {
        if (e.isJsonArray()) {
            for (JsonElement x : e.getAsJsonArray()) walk(x, ops);
            return;
        }
        if (!e.isJsonObject()) return;
        JsonObject o = e.getAsJsonObject();
        JsonElement type = o.get("type");
        if (type != null && type.isJsonPrimitive() && o.has("predicate")) {
            String t = type.getAsString();
            JsonElement p = o.get("predicate");
            switch (t) {
                case "minecraft:entity_properties" -> {
                    String target = o.get("entity").getAsString();
                    String key = target + "|" + canonical(p);
                    ENTITY_PREDS.computeIfAbsent(key, k -> new EntityPred(target, key, EntityPredicate.CODEC.parse(ops, p).getOrThrow()));
                }
                case "minecraft:damage_source_properties" -> {
                    String key = canonical(p);
                    DAMAGE_PREDS.computeIfAbsent(key, k -> new DamagePred(key, DamageSourcePredicate.CODEC.parse(ops, p).getOrThrow()));
                }
                case "minecraft:location_check" -> {
                    int[] off = {intOr(o, "offsetX"), intOr(o, "offsetY"), intOr(o, "offsetZ")};
                    String key = canonical(p) + "|" + off[0] + "," + off[1] + "," + off[2];
                    LOCATION_PREDS.computeIfAbsent(key, k -> new LocationPred(key, off, LocationPredicate.CODEC.parse(ops, p).getOrThrow()));
                }
                default -> {}
            }
        }
        for (var entry : o.entrySet()) walk(entry.getValue(), ops);
    }

    static int intOr(JsonObject o, String key) {
        return o.has(key) ? o.get(key).getAsInt() : 0;
    }

    // ---- contexts --------------------------------------------------------------------------------

    /** One evaluation context and what it records. */
    static class Ctx {
        Vec3 origin = new Vec3(8.5, 64.0, 8.5);
        ItemStack tool;
        BlockState state;
        Float explosion;
        float luck;
        Map<String, Entity> entities = new LinkedHashMap<>();
        DamageSource damage;
        BlockEntity blockEntity;

        LootParams params(ContextKeySet set) {
            var b = new LootParams.Builder(LEVEL).withLuck(luck);
            b.withOptionalParameter(LootContextParams.ORIGIN, origin);
            if (tool != null) b.withOptionalParameter(LootContextParams.TOOL, tool);
            if (state != null) b.withOptionalParameter(LootContextParams.BLOCK_STATE, state);
            if (explosion != null) b.withOptionalParameter(LootContextParams.EXPLOSION_RADIUS, explosion);
            if (damage != null) b.withOptionalParameter(LootContextParams.DAMAGE_SOURCE, damage);
            if (blockEntity != null) b.withOptionalParameter(LootContextParams.BLOCK_ENTITY, blockEntity);
            for (var e : entities.entrySet()) {
                switch (e.getKey()) {
                    case "this" -> b.withOptionalParameter(LootContextParams.THIS_ENTITY, e.getValue());
                    case "attacker" -> b.withOptionalParameter(LootContextParams.ATTACKING_ENTITY, e.getValue());
                    case "direct_attacker" -> b.withOptionalParameter(LootContextParams.DIRECT_ATTACKING_ENTITY, e.getValue());
                    case "attacking_player" -> b.withOptionalParameter(LootContextParams.LAST_DAMAGE_PLAYER, (net.minecraft.world.entity.player.Player) e.getValue());
                    case "target_entity" -> b.withOptionalParameter(LootContextParams.TARGET_ENTITY, e.getValue());
                    case "interacting_entity" -> b.withOptionalParameter(LootContextParams.INTERACTING_ENTITY, e.getValue());
                    default -> throw new IllegalStateException(e.getKey());
                }
            }
            return b.create(set);
        }

        /** Adds defaults for required parameters the kind-specific setup did not provide. */
        void fillRequired(ContextKeySet set) throws Exception {
            var req = set.required();
            if (req.contains(LootContextParams.TOOL) && tool == null) tool = new ItemStack(Items.BRUSH);
            if (req.contains(LootContextParams.BLOCK_STATE) && state == null) state = Blocks.STONE.defaultBlockState();
            if (req.contains(LootContextParams.DAMAGE_SOURCE) && damage == null) damage = DAMAGE.generic();
            if (req.contains(LootContextParams.THIS_ENTITY)) entities.putIfAbsent("this", create(EntityTypes.PIG));
            if (req.contains(LootContextParams.TARGET_ENTITY)) entities.putIfAbsent("target_entity", create(EntityTypes.ARMADILLO));
            if (req.contains(LootContextParams.INTERACTING_ENTITY)) entities.putIfAbsent("interacting_entity", player());
            if (req.contains(LootContextParams.ATTACKING_ENTITY)) entities.putIfAbsent("attacker", create(EntityTypes.ZOMBIE));
        }

        /** Drops the parameters the table's parameter set does not allow (LootParams rejects them). */
        void prune(ContextKeySet set) {
            var allowed = set.allowed();
            if (!allowed.contains(LootContextParams.TOOL)) tool = null;
            if (!allowed.contains(LootContextParams.BLOCK_STATE)) state = null;
            if (!allowed.contains(LootContextParams.EXPLOSION_RADIUS)) explosion = null;
            if (!allowed.contains(LootContextParams.DAMAGE_SOURCE)) damage = null;
            if (!allowed.contains(LootContextParams.BLOCK_ENTITY)) blockEntity = null;
            if (!allowed.contains(LootContextParams.ORIGIN)) origin = null;
            entities.keySet().removeIf(k -> !allowed.contains(switch (k) {
                case "this" -> LootContextParams.THIS_ENTITY;
                case "attacker" -> LootContextParams.ATTACKING_ENTITY;
                case "direct_attacker" -> LootContextParams.DIRECT_ATTACKING_ENTITY;
                case "attacking_player" -> LootContextParams.LAST_DAMAGE_PLAYER;
                case "target_entity" -> LootContextParams.TARGET_ENTITY;
                default -> LootContextParams.INTERACTING_ENTITY;
            }));
            entities.values().removeIf(java.util.Objects::isNull);
        }

        void setupWorld() {
            WORLD.clear();
            if (state != null && origin != null) {
                BlockPos at = BlockPos.containing(origin);
                WORLD.put(at, state);
                // Double plants check their other half.
                WORLD.put(at.above(), state);
                WORLD.put(at.below(), state);
            }
        }

        String json() {
            List<String> parts = new ArrayList<>();
            if (origin != null) parts.add("\"origin\": [" + origin.x + ", " + origin.y + ", " + origin.z + "]");
            if (tool != null) parts.add("\"tool\": \"" + hex(tool) + "\"");
            if (state != null) parts.add("\"block_state\": " + Block.getId(state));
            if (explosion != null) parts.add("\"explosion_radius\": " + explosion);
            parts.add("\"luck\": " + luck);
            if (damage != null) parts.add("\"damage_source\": true");
            List<String> ents = new ArrayList<>();
            var enchantments = access.lookupOrThrow(Registries.ENCHANTMENT);
            for (var e : entities.entrySet()) {
                List<String> levels = new ArrayList<>();
                if (e.getValue() instanceof LivingEntity living && !(e.getValue() instanceof FakePlayer)) {
                    enchantments.listElements().forEach(h -> {
                        int lvl = EnchantmentHelper.getEnchantmentLevel(h, living);
                        if (lvl > 0) levels.add(str(h.key().identifier().toString()) + ": " + lvl);
                    });
                }
                String profile = "";
                if (e.getValue() instanceof FakePlayer fp) {
                    var resolved = net.minecraft.world.item.component.ResolvableProfile.createResolved(fp.getGameProfile());
                    profile = ", \"profile\": \"" + hex(new TypedDataComponent<>(DataComponents.PROFILE, resolved)) + "\"";
                }
                ents.add(str(e.getKey()) + ": {\"type\": " + str(BuiltInRegistries.ENTITY_TYPE.getKey(e.getValue().getType()).toString())
                        + ", \"enchantments\": {" + String.join(", ", levels) + "}" + profile + "}");
            }
            parts.add("\"entities\": {" + String.join(", ", ents) + "}");
            if (blockEntity != null) {
                List<String> comps = new ArrayList<>();
                DataComponentMap map = blockEntity.collectComponents();
                for (TypedDataComponent<?> c : map) comps.add("\"" + hex(c) + "\"");
                String name = "null";
                if (blockEntity instanceof Nameable n) {
                    Component custom = n.getCustomName();
                    name = custom == null ? "\"\"" : "\"" + hexText(custom) + "\"";
                }
                parts.add("\"block_entity\": {\"components\": [" + String.join(", ", comps) + "], \"name\": " + name + "}");
            }
            // Vanilla's answers to the world predicates, for kiln's context hooks.
            List<String> answers = new ArrayList<>();
            for (EntityPred p : ENTITY_PREDS.values()) {
                Entity e = entities.get(p.target());
                if (e == null) continue;
                try {
                    answers.add(str(p.key()) + ": " + p.predicate().matches(LEVEL, origin, e));
                } catch (Throwable t) {
                    // A fake entity lacking the state the predicate reads: leave it unanswered.
                }
            }
            parts.add("\"entity_predicates\": {" + String.join(", ", answers) + "}");
            answers.clear();
            if (damage != null) {
                for (DamagePred p : DAMAGE_PREDS.values()) answers.add(str(p.key()) + ": " + p.predicate().matches(LEVEL, origin, damage));
            }
            parts.add("\"damage_predicates\": {" + String.join(", ", answers) + "}");
            answers.clear();
            for (LocationPred p : origin == null ? List.<LocationPred>of() : LOCATION_PREDS.values()) {
                int[] o = p.offset();
                answers.add(str(p.key()) + ": " + p.predicate().matches(LEVEL, origin.x + o[0], origin.y + o[1], origin.z + o[2]));
            }
            parts.add("\"location_predicates\": {" + String.join(", ", answers) + "}");
            return "{" + String.join(", ", parts) + "}";
        }
    }

    static Holder<Enchantment> ench(ResourceKey<Enchantment> key) {
        return access.lookupOrThrow(Registries.ENCHANTMENT).getOrThrow(key);
    }

    static ItemStack tool(Item item, Object... enchants) {
        ItemStack s = new ItemStack(item);
        for (int i = 0; i < enchants.length; i += 2) {
            @SuppressWarnings("unchecked")
            ResourceKey<Enchantment> k = (ResourceKey<Enchantment>) enchants[i];
            s.enchant(ench(k), (Integer) enchants[i + 1]);
        }
        return s;
    }

    static List<ItemStack> blockTools() {
        return List.of(
                ItemStack.EMPTY,
                tool(Items.DIAMOND_PICKAXE),
                tool(Items.DIAMOND_PICKAXE, Enchantments.FORTUNE, 1),
                tool(Items.NETHERITE_PICKAXE, Enchantments.FORTUNE, 3),
                tool(Items.IRON_PICKAXE, Enchantments.SILK_TOUCH, 1),
                tool(Items.SHEARS),
                tool(Items.SHEARS, Enchantments.SILK_TOUCH, 1),
                tool(Items.DIAMOND_AXE, Enchantments.FORTUNE, 2),
                tool(Items.DIAMOND_HOE),
                tool(Items.GOLDEN_HOE, Enchantments.FORTUNE, 3),
                tool(Items.DIAMOND_SHOVEL, Enchantments.SILK_TOUCH, 1),
                tool(Items.STICK));
    }

    static Entity create(EntityType<?> type) {
        try {
            Entity e = type.create(LEVEL, EntitySpawnReason.COMMAND);
            if (e != null) e.setPos(8.5, 64.0, 8.5);
            return e;
        } catch (Throwable t) {
            if (CREATE_ERRORS++ < 5) t.printStackTrace(OUT);
            return null;
        }
    }

    static int CREATE_ERRORS;

    static FakePlayer player() throws Exception {
        FakePlayer p = (FakePlayer) U.allocateInstance(FakePlayer.class);
        set(p, Entity.class, "type", EntityTypes.PLAYER);
        set(p, net.minecraft.world.entity.player.Player.class, "gameProfile", new com.mojang.authlib.GameProfile(new java.util.UUID(1, 2), "kiln"));
        return p;
    }

    static LivingEntity armed(EntityType<?> type, ItemStack weapon) {
        Entity e = create(type);
        if (e instanceof LivingEntity l) {
            l.setItemSlot(EquipmentSlot.MAINHAND, weapon);
            return l;
        }
        return null;
    }

    // ---- cases -----------------------------------------------------------------------------------

    record Case(String mode, long worldSeed, long seed, int runs) {}

    static void run(Path out, int contexts, long seed) throws Exception {
        var lookup = resources.fullRegistries().lookup().lookupOrThrow(Registries.LOOT_TABLE);
        List<ResourceKey<LootTable>> keys = new ArrayList<>(lookup.listElementIds().toList());
        keys.sort((a, b) -> a.identifier().toString().compareTo(b.identifier().toString()));
        Map<String, LootTable> tables = new LinkedHashMap<>();
        for (ResourceKey<LootTable> key : keys) tables.put(key.identifier().toString(), resources.fullRegistries().getLootTable(key));
        run(out, contexts, seed, tables, null);
    }

    /** Hand-written tables (a JSON object of id to table) decoded with vanilla's codec. */
    static void synthetic(Path in, Path out, int contexts, long seed) throws Exception {
        var ops = resources.fullRegistries().lookup().createSerializationContext(JsonOps.INSTANCE);
        JsonObject all = JsonParser.parseString(Files.readString(in)).getAsJsonObject();
        walk(all, access.createSerializationContext(JsonOps.INSTANCE));
        Map<String, LootTable> tables = new LinkedHashMap<>();
        for (var e : all.entrySet()) tables.put(e.getKey(), LootTable.DIRECT_CODEC.parse(ops, e.getValue()).getOrThrow());
        run(out, contexts, seed, tables, "synthetic");
    }

    static void run(Path out, int contexts, long seed, Map<String, LootTable> tables, String file) throws Exception {
        Random rnd = new Random(seed);
        Map<String, PrintWriter> writers = new HashMap<>();
        Map<String, int[]> counts = new TreeMap<>();
        for (var entry : tables.entrySet()) {
            LootTable table = entry.getValue();
            String kind = kind(table.getParamSet());
            String id = entry.getKey();
            PrintWriter w = writers.computeIfAbsent(file != null ? file : kind, k -> {
                try {
                    return new PrintWriter(Files.newBufferedWriter(out.resolve(k + ".jsonl")));
                } catch (Exception ex) {
                    throw new RuntimeException(ex);
                }
            });
            int[] c = counts.computeIfAbsent(kind, k -> new int[2]);
            for (int i = 0; i < contexts; i++) {
                Ctx ctx;
                try {
                    ctx = context(kind, id, i, rnd);
                } catch (Throwable t) {
                    c[1]++;
                    continue;
                }
                if (ctx == null) continue;
                try {
                    ctx.fillRequired(table.getParamSet());
                } catch (Throwable t) {
                    c[1]++;
                    continue;
                }
                ctx.prune(table.getParamSet());
                ctx.setupWorld();
                String ctxJson;
                try {
                    ctxJson = ctx.json();
                } catch (Throwable t) {
                    c[1]++;
                    continue;
                }
                List<Case> cases = new ArrayList<>();
                cases.add(new Case("sequence", rnd.nextLong(), 0, 3));
                cases.add(new Case("seed", 0, rnd.nextLong() | 1, 1));
                if (kind.equals("chest")) cases.add(new Case("fill", 0, rnd.nextLong() | 1, 1));
                for (Case cs : cases) {
                    String results;
                    try {
                        results = evaluate(table, ctx, cs);
                    } catch (Throwable t) {
                        c[1]++;
                        OUT.println("error " + id + " " + cs.mode() + ": " + t);
                        continue;
                    }
                    w.println("{\"table\": " + str(id) + ", \"kind\": " + str(kind) + ", \"mode\": " + str(cs.mode())
                            + ", \"world_seed\": " + cs.worldSeed() + ", \"seed\": " + cs.seed() + ", \"context\": " + ctxJson
                            + ", \"results\": " + results + "}");
                    c[0]++;
                }
            }
        }
        for (PrintWriter w : writers.values()) w.close();
        for (var e : counts.entrySet()) OUT.println(e.getKey() + ": " + e.getValue()[0] + " cases, " + e.getValue()[1] + " skipped");
    }

    static String evaluate(LootTable table, Ctx ctx, Case cs) {
        LootParams params = ctx.params(table.getParamSet());
        List<String> runs = new ArrayList<>();
        switch (cs.mode()) {
            case "sequence" -> {
                SEQS = new RandomSequences();
                WORLD_SEED = cs.worldSeed();
                // Tables without a random sequence draw from the level's random.
                try {
                    set(LEVEL, Level.class, "random", RandomSource.create(cs.worldSeed()));
                } catch (Exception e) {
                    throw new RuntimeException(e);
                }
                for (int r = 0; r < cs.runs(); r++) runs.add(list(table.getRandomItems(params)));
            }
            case "seed" -> runs.add(list(table.getRandomItems(params, cs.seed())));
            case "fill" -> {
                SimpleContainer container = new SimpleContainer(27);
                container.setItem(4, new ItemStack(Items.STONE));
                table.fill(container, params, cs.seed());
                List<ItemStack> slots = new ArrayList<>();
                for (int i = 0; i < container.getContainerSize(); i++) slots.add(container.getItem(i));
                runs.add(list(slots));
            }
            default -> throw new IllegalStateException(cs.mode());
        }
        return "[" + String.join(", ", runs) + "]";
    }

    static String list(List<ItemStack> stacks) {
        List<String> out = new ArrayList<>();
        for (ItemStack s : stacks) out.add("\"" + hex(s) + "\"");
        return "[" + String.join(", ", out) + "]";
    }

    static String kind(ContextKeySet set) {
        if (set == LootContextParamSets.BLOCK) return "block";
        if (set == LootContextParamSets.ENTITY) return "entity";
        if (set == LootContextParamSets.CHEST) return "chest";
        if (set == LootContextParamSets.FISHING) return "fishing";
        if (set == LootContextParamSets.GIFT) return "gift";
        if (set == LootContextParamSets.ARCHAEOLOGY) return "archaeology";
        if (set == LootContextParamSets.SHEARING) return "shearing";
        if (set == LootContextParamSets.PIGLIN_BARTER) return "barter";
        if (set == LootContextParamSets.EQUIPMENT) return "equipment";
        if (set == LootContextParamSets.BLOCK_INTERACT) return "block_interact";
        if (set == LootContextParamSets.ENTITY_INTERACT) return "entity_interact";
        if (set == LootContextParamSets.VAULT) return "vault";
        return "other";
    }

    static final List<ResourceKey<Enchantment>> LOOTING = List.of(Enchantments.LOOTING);

    static Ctx context(String kind, String id, int i, Random rnd) throws Exception {
        Ctx c = new Ctx();
        String path = id.substring(id.indexOf(':') + 1);
        switch (kind) {
            case "block" -> {
                String name = path.startsWith("blocks/") ? path.substring(7) : path;
                if (id.startsWith("kiln:") && name.contains("/")) name = name.substring(0, name.indexOf('/'));
                Block block = BuiltInRegistries.BLOCK.getOptional(Identifier.withDefaultNamespace(name)).orElse(Blocks.STONE);
                List<BlockState> states = block.getStateDefinition().getPossibleStates();
                c.state = i == 0 ? block.defaultBlockState() : states.get(rnd.nextInt(states.size()));
                List<ItemStack> tools = blockTools();
                c.tool = tools.get(i < tools.size() ? i : rnd.nextInt(tools.size())).copy();
                c.explosion = rnd.nextInt(4) == 0 ? 2.0f + rnd.nextInt(4) : null;
                if (block instanceof EntityBlock eb) {
                    BlockEntity be = eb.newBlockEntity(BlockPos.containing(c.origin), c.state);
                    if (be != null) {
                        be.setLevel(LEVEL);
                        if (rnd.nextBoolean()) {
                            be.applyComponents(DataComponentMap.builder().set(DataComponents.CUSTOM_NAME, Component.literal("Named " + i)).build(),
                                    DataComponentPatch.EMPTY);
                        }
                        c.blockEntity = be;
                    }
                }
                if (rnd.nextInt(3) == 0) c.entities.put("this", player());
            }
            case "entity" -> {
                String rest = path.startsWith("entities/") ? path.substring(9) : path.startsWith("entity/") ? path.substring(7) : path;
                String typeName = rest.contains("/") ? rest.substring(0, rest.indexOf('/')) : rest;
                EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getOptional(Identifier.withDefaultNamespace(typeName)).orElse(EntityTypes.PIG);
                Entity self = create(type);
                if (self == null) self = create(EntityTypes.PIG);
                if (self instanceof Sheep sheep && rest.contains("/")) {
                    DyeColor color = DyeColor.byName(rest.substring(rest.indexOf('/') + 1), null);
                    if (color != null) sheep.setColor(color);
                }
                if (rnd.nextInt(3) == 0) self.setRemainingFireTicks(100);
                c.entities.put("this", self);
                int variant = i % 6;
                switch (variant) {
                    case 0 -> c.damage = DAMAGE.generic();
                    case 1, 2, 3 -> {
                        int looting = rnd.nextInt(4);
                        ItemStack sword = looting > 0 ? tool(Items.DIAMOND_SWORD, Enchantments.LOOTING, looting) : new ItemStack(Items.DIAMOND_SWORD);
                        if (variant == 3) sword.enchant(ench(Enchantments.FIRE_ASPECT), 1);
                        LivingEntity killer = armed(variant == 2 ? EntityTypes.SKELETON : EntityTypes.ZOMBIE, sword);
                        c.entities.put("attacker", killer);
                        c.entities.put("direct_attacker", killer);
                        c.damage = DAMAGE.mobAttack(killer);
                        if (rnd.nextBoolean()) c.entities.put("attacking_player", player());
                    }
                    case 4 -> {
                        c.damage = DAMAGE.lightningBolt();
                        c.entities.put("attacking_player", player());
                    }
                    default -> {
                        LivingEntity frog = (LivingEntity) create(EntityTypes.FROG);
                        c.entities.put("attacker", frog);
                        c.entities.put("direct_attacker", frog);
                        c.damage = DAMAGE.mobAttack(frog);
                    }
                }
            }
            default -> {
                c.luck = (i % 3 == 2) ? 1.5f : 0f;
                switch (kind) {
                    case "fishing" -> {
                        c.tool = rnd.nextBoolean() ? tool(Items.FISHING_ROD, Enchantments.LUCK_OF_THE_SEA, 1 + rnd.nextInt(3)) : new ItemStack(Items.FISHING_ROD);
                        Entity hook = create(EntityTypes.PIG);
                        c.entities.put("this", hook);
                    }
                    case "shearing" -> {
                        c.tool = new ItemStack(Items.SHEARS);
                        String rest = path.startsWith("shearing/") ? path.substring(9) : path;
                        String typeName = rest.contains("/") ? rest.substring(0, rest.indexOf('/')) : rest;
                        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getOptional(Identifier.withDefaultNamespace(typeName)).orElse(EntityTypes.SHEEP);
                        Entity e = create(type);
                        c.entities.put("this", e == null ? create(EntityTypes.SHEEP) : e);
                    }
                    case "archaeology" -> {
                        c.tool = new ItemStack(Items.BRUSH);
                        c.entities.put("this", player());
                    }
                    case "gift", "barter", "equipment", "entity_interact" -> c.entities.put("this", create(EntityTypes.PIG));
                    case "block_interact" -> {
                        c.tool = new ItemStack(Items.SHEARS);
                        c.state = Blocks.PUMPKIN.defaultBlockState();
                        c.entities.put("this", player());
                    }
                    case "chest" -> {
                        if (rnd.nextBoolean()) c.entities.put("this", player());
                    }
                    default -> {}
                }
            }
        }
        return c;
    }
}
