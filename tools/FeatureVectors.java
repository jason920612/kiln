// Runs vanilla 26.3's overworld generation through FEATURES for sets of chunks, in Kiln's
// canonical decoration order, and dumps what every placed feature (and structure piece
// placement) wrote, for kiln-worldgen's feature parity test.
//
// The harness starts a real dedicated server in-process (cwd must hold eula.txt and a
// server.properties with the seed; tools/feature_vectors.py prepares it) and uses its
// overworld only as generation context: seed, registries, structure and template managers,
// POI bookkeeping. The chunks themselves are the harness's own ProtoChunks: BIOMES (canonical,
// empty R-tree history), TERRAIN (fill, surface, carvers), then FEATURES through a real
// WorldGenRegion (allocated without its constructor so it can wrap these chunks; light reads
// return 0 as for unlit proto-chunks) with a copy of ChunkGenerator.applyBiomeDecoration's loop
// that records the blocks changed by each placed feature.
//
// Canonical order (see kiln-worldgen pipeline.rs): a chunk is decorated after every chunk
// within Chebyshev distance 2 whose rank is lower; rank = RANK[(x mod 3) * 3 + (z mod 3)].
// For each target region the harness decorates the closure of the target chunks' 3x3
// neighbourhoods under that rule, in (rank, x, z) order.
//
// Output per seed, features_<seed>.bin:
//   header   "KWGF", version, seed, structures flag, block state count, min y, height,
//            feature steps: count, per step the placed feature ids in FeatureSorter order
//            (inline features are "#<step>:<index>"), then regions
//   region   target count + (x, z)...; with structures: chunks with structure starts (x, z,
//            start count, per start the structure id and its saved NBT as length + bytes),
//            chunks with references (x, z, structure count, per structure id, count, packed
//            chunk positions as i64) and the post-TERRAIN blocks of each decorated chunk (x, z,
//            deflated blocks); then decoration count and per decorated chunk in order:
//            x, z, deflated record: invocation count, per invocation kind (0 feature,
//            1 structure), step, index, far reads, change count, changes (dx+16, dz+16 as u8,
//            y as i16, state as u16; dx, dz relative to the chunk's origin);
//            then per target chunk: x, z, deflated final blocks (u16, Kiln section order).
//
// usage (through tools/feature_vectors.py):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/FeatureVectors.java <out dir> <seed> [--regions N] [--size S] [--check]
//        [--bench N]

import java.io.BufferedOutputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintStream;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Supplier;
import java.util.stream.Collectors;
import java.util.zip.Deflater;
import it.unimi.dsi.fastutil.ints.IntArraySet;
import it.unimi.dsi.fastutil.ints.IntSet;
import it.unimi.dsi.fastutil.objects.ObjectArraySet;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderSet;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.SectionPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.WorldGenRegion;
import net.minecraft.util.StaticCache2D;
import net.minecraft.world.attribute.EnvironmentAttributeSystem;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeGenerationSettings;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.biome.BiomeResolver;
import net.minecraft.world.level.biome.Climate;
import net.minecraft.world.level.biome.FeatureSorter;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.chunk.status.ChunkPyramid;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.Aquifer;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.GenerationStep;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseChunk;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.RandomSupport;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;
import net.minecraft.world.level.levelgen.material.rule.MaterialRule;
import net.minecraft.world.level.levelgen.placement.FeaturePlacer;
import net.minecraft.world.level.levelgen.placement.PlacedFeature;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.lighting.LevelLightEngine;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.ticks.WorldGenTickAccess;

public class FeatureVectors {
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);

    /** Decoration rank of a chunk by (x mod 3, z mod 3); must match kiln-worldgen's pipeline. */
    static final int[] RANK = {0, 1, 2, 7, 6, 3, 8, 5, 4};

    static int rank(int x, int z) {
        return RANK[Math.floorMod(x, 3) * 3 + Math.floorMod(z, 3)];
    }

    // Target regions (origin chunk); the rest are drawn from a seeded Random.
    static final int[][] FIXED_REGIONS = {
        {0, 0}, {-40, 24}, {100, -300}, {-2000, 1500}, {10000, 10000}, {-62500, 62500},
        {1048572, -1048580}, {123456, -654321},
    };

    final MinecraftServer server;
    final ServerLevel level;
    final RegistryAccess registries;
    final NoiseBasedChunkGenerator generator;
    final RandomState random;
    final NoiseGeneratorSettings settings;
    final MultiNoiseBiomeSource biomeSource;
    final PalettedContainerFactory containers;
    final boolean structures;
    final long seed;
    final ThreadLocal<?> lastResult;
    final Aquifer.FluidPicker fluidPicker;
    final Method doFill, buildSurface, generateCarvers;
    final List<FeatureSorter.StepFeatureData> steps;
    final java.util.function.Function<Holder<Biome>, BiomeGenerationSettings> settingsGetter;
    final Map<Long, Holder<Biome>[]> biomes = new ConcurrentHashMap<>();
    final BiomeManager biomeManager;
    final Identifier regionRandomId;
    final Field sectionData;
    final Method dataStorage;

    @SuppressWarnings("unchecked")
    FeatureVectors(MinecraftServer server) throws Exception {
        this.server = server;
        level = server.overworld();
        registries = server.registryAccess();
        generator = (NoiseBasedChunkGenerator) level.getChunkSource().getGenerator();
        random = level.getChunkSource().randomState();
        settings = generator.generatorSettings().value();
        biomeSource = (MultiNoiseBiomeSource) generator.getBiomeSource();
        containers = PalettedContainerFactory.create(registries);
        structures = server.overworld().structureManager().shouldGenerateStructures();
        seed = level.getSeed();
        biomeManager = new BiomeManager(this::storedBiome, BiomeManager.obfuscateSeed(seed));

        Method parameters = MultiNoiseBiomeSource.class.getDeclaredMethod("parameters");
        parameters.setAccessible(true);
        Object list = parameters.invoke(biomeSource);
        Field index = Climate.ParameterList.class.getDeclaredField("index");
        index.setAccessible(true);
        Object tree = index.get(list);
        Field last = tree.getClass().getDeclaredField("lastResult");
        last.setAccessible(true);
        lastResult = (ThreadLocal<?>) last.get(tree);

        Field picker = NoiseBasedChunkGenerator.class.getDeclaredField("globalFluidPicker");
        picker.setAccessible(true);
        fluidPicker = ((Supplier<Aquifer.FluidPicker>) picker.get(generator)).get();
        doFill = NoiseBasedChunkGenerator.class.getDeclaredMethod("doFill", NoiseChunk.class, ChunkAccess.class);
        buildSurface = NoiseBasedChunkGenerator.class.getDeclaredMethod("buildSurface", ChunkAccess.class, NoiseChunk.class,
            RandomState.class, BiomeManager.class, Set.class, MaterialRule.class);
        generateCarvers = NoiseBasedChunkGenerator.class.getDeclaredMethod("generateCarvers", ChunkAccess.class, Blender.class,
            NoiseChunk.class, RandomState.class, BiomeManager.class, WorldGenRegion.class, MaterialRule.class);
        for (Method m : new Method[] {doFill, buildSurface, generateCarvers}) m.setAccessible(true);

        Field fps = net.minecraft.world.level.chunk.ChunkGenerator.class.getDeclaredField("featuresPerStep");
        fps.setAccessible(true);
        steps = ((Supplier<List<FeatureSorter.StepFeatureData>>) fps.get(generator)).get();
        Field gsg = net.minecraft.world.level.chunk.ChunkGenerator.class.getDeclaredField("generationSettingsGetter");
        gsg.setAccessible(true);
        settingsGetter = (java.util.function.Function<Holder<Biome>, BiomeGenerationSettings>) gsg.get(generator);

        Field rr = WorldGenRegion.class.getDeclaredField("WORLDGEN_REGION_RANDOM");
        rr.setAccessible(true);
        regionRandomId = (Identifier) rr.get(null);

        sectionData = PalettedContainer.class.getDeclaredField("data");
        sectionData.setAccessible(true);
        Class<?> dataClass = Class.forName("net.minecraft.world.level.chunk.PalettedContainer$Data");
        dataStorage = dataClass.getDeclaredMethod("storage");
        dataStorage.setAccessible(true);
    }

    int minY() {
        return level.getMinY();
    }

    int sections() {
        return level.getSectionsCount();
    }

    // ---- BIOMES (canonical) -------------------------------------------------------------

    Holder<Biome>[] chunkBiomes(int cx, int cz) {
        return biomes.computeIfAbsent(ChunkPos.pack(cx, cz), k -> computeBiomes(cx, cz));
    }

    @SuppressWarnings("unchecked")
    Holder<Biome>[] computeBiomes(int cx, int cz) {
        lastResult.remove();
        SamplerContext ctx = SamplerContext.builder().enableCaches().build();
        int qx = cx * 4, qz = cz * 4, qy = minY() >> 2;
        BiomeResolver resolver = biomeSource.createResolverForChunk(random.createClimateSampler(ctx), qx, qy, qz, 4,
            level.getHeight() >> 2, 4);
        Holder<Biome>[] out = new Holder[sections() * 64];
        for (int s = 0; s < sections(); s++) {
            for (int x = 0; x < 4; x++) {
                for (int y = 0; y < 4; y++) {
                    for (int z = 0; z < 4; z++) {
                        out[s * 64 + (y << 4 | z << 2 | x)] = resolver.getNoiseBiome(qx + x, qy + s * 4 + y, qz + z);
                    }
                }
            }
        }
        return out;
    }

    Holder<Biome> storedBiome(int qx, int qy, int qz) {
        int s = Math.clamp((qy - (minY() >> 2)) >> 2, 0, sections() - 1);
        int ly = Math.clamp(qy - (minY() >> 2) - s * 4, 0, 3);
        return chunkBiomes(qx >> 2, qz >> 2)[s * 64 + (ly << 4 | (qz & 3) << 2 | (qx & 3))];
    }

    // ---- TERRAIN ------------------------------------------------------------------------

    ProtoChunk terrain(int cx, int cz) throws Exception {
        ProtoChunk chunk = new ProtoChunk(new ChunkPos(cx, cz), UpgradeData.EMPTY, level, containers, null);
        terrain(chunk, Beardifier.EMPTY);
        return chunk;
    }

    /** BIOMES and TERRAIN on a chunk (with the structures' beardifier, if any). */
    void terrain(ProtoChunk chunk, Beardifier beardifier) throws Exception {
        ChunkPos pos = chunk.getPos();
        Holder<Biome>[] b = chunkBiomes(pos.x(), pos.z());
        int minQy = minY() >> 2;
        chunk.fillBiomesFromNoise((x, y, z) -> b[((y - minQy) >> 2) * 64 + (((y - minQy) & 3) << 4 | (z & 3) << 2 | (x & 3))]);
        chunk.setPersistedStatus(ChunkStatus.BIOMES);
        DensityVolume volume = new DensityVolume(16, level.getHeight(), 16, pos.getMinBlockX(), minY(), pos.getMinBlockZ());
        try (NoiseChunk noise = new NoiseChunk(random, beardifier, settings, fluidPicker, Blender.empty(), volume)) {
            doFill.invoke(generator, noise, chunk);
            buildSurface.invoke(generator, chunk, noise, random, biomeManager, biomeSource.possibleBiomes(), settings.materialRule().value());
            generateCarvers.invoke(generator, chunk, Blender.empty(), noise, random, biomeManager, null, settings.materialRule().value());
        }
        chunk.setPersistedStatus(ChunkStatus.TERRAIN);
    }

    // ---- FEATURES -----------------------------------------------------------------------

    /** WorldGenRegion without its constructor; unlit like a proto-chunk mid-generation. */
    static final class HarnessRegion extends WorldGenRegion {
        long farReads;
        int cx, cz;

        HarnessRegion() {
            super(null, null, null, null);
        }

        @Override
        public LevelLightEngine getLightEngine() {
            return LevelLightEngine.EMPTY;
        }

        void countFar(BlockPos pos) {
            if (Math.max(Math.abs((pos.getX() >> 4) - cx), Math.abs((pos.getZ() >> 4) - cz)) > 1) farReads++;
        }

        @Override
        public BlockState getBlockState(BlockPos pos) {
            countFar(pos);
            return super.getBlockState(pos);
        }

        @Override
        public FluidState getFluidState(BlockPos pos) {
            countFar(pos);
            return super.getFluidState(pos);
        }
    }

    static final sun.misc.Unsafe UNSAFE;

    static {
        try {
            Field f = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
            f.setAccessible(true);
            UNSAFE = (sun.misc.Unsafe) f.get(null);
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        }
    }

    static void set(Object target, String name, Object value) throws ReflectiveOperationException {
        Field f = WorldGenRegion.class.getDeclaredField(name);
        f.setAccessible(true);
        if (f.getType() == int.class) f.setInt(target, (Integer) value);
        else if (f.getType() == long.class) f.setLong(target, (Long) value);
        else f.set(target, value);
    }

    HarnessRegion region(ProtoChunk center, Map<Long, ProtoChunk> chunks) throws Exception {
        return region(center, chunks, ChunkStatus.FEATURES);
    }

    HarnessRegion region(ProtoChunk center, Map<Long, ProtoChunk> chunks, ChunkStatus status) throws Exception {
        HarnessRegion r = (HarnessRegion) UNSAFE.allocateInstance(HarnessRegion.class);
        ChunkPos c = center.getPos();
        r.cx = c.x();
        r.cz = c.z();
        var step = ChunkPyramid.GENERATION_PYRAMID.getStepTo(status);
        int radius = step.directDependencies().size() - 1;
        StaticCache2D<ChunkAccess> cache = StaticCache2D.create(c.x(), c.z(), radius, (x, z) -> {
            ProtoChunk p = chunks.get(ChunkPos.pack(x, z));
            return p != null ? p : new ProtoChunk(new ChunkPos(x, z), UpgradeData.EMPTY, level, containers, null);
        });
        set(r, "blockTicks", new WorldGenTickAccess<Block>(pos -> r.getChunk(pos).getBlockTicks()));
        set(r, "fluidTicks", new WorldGenTickAccess<net.minecraft.world.level.material.Fluid>(pos -> r.getChunk(pos).getFluidTicks()));
        set(r, "subTickCount", new AtomicLong());
        set(r, "generatingStep", step);
        set(r, "cache", cache);
        set(r, "center", center);
        set(r, "level", level);
        set(r, "seed", seed);
        set(r, "levelData", level.getLevelData());
        set(r, "random", random.getOrCreateRandomFactory(regionRandomId).at(c.getWorldPosition()));
        set(r, "dimensionType", level.dimensionType());
        set(r, "uncachedBiomeResolver", level.uncachedBiomeResolver());
        set(r, "biomeManager", new BiomeManager(r, BiomeManager.obfuscateSeed(seed)));
        set(r, "centerChunkX", c.x());
        set(r, "centerChunkZ", c.z());
        set(r, "writeRadius", step.blockStateWriteRadius());
        set(r, "environmentAttributes", EnvironmentAttributeSystem.builder().addStaticLayers(r).build());
        return r;
    }

    /** Block state snapshots of the 3x3 chunks around a decorated chunk, for per-feature diffs. */
    final class Shadow {
        final ProtoChunk[] chunks = new ProtoChunk[9];
        final Object[][] data;
        final long[][][] raw;
        final BlockState[][][] states;
        final int cx, cz;

        Shadow(int cx, int cz, Map<Long, ProtoChunk> all) throws Exception {
            this.cx = cx;
            this.cz = cz;
            int n = sections();
            data = new Object[9][n];
            raw = new long[9][n][];
            states = new BlockState[9][n][4096];
            for (int i = 0; i < 9; i++) {
                chunks[i] = all.get(ChunkPos.pack(cx + i % 3 - 1, cz + i / 3 - 1));
                for (int s = 0; s < n; s++) {
                    LevelChunkSection section = chunks[i].getSection(s);
                    Object d = sectionData.get(section.getStates());
                    data[i][s] = d;
                    raw[i][s] = ((net.minecraft.util.BitStorage) dataStorage.invoke(d)).getRaw().clone();
                    for (int k = 0; k < 4096; k++) states[i][s][k] = section.getBlockState(k & 15, k >> 8, (k >> 4) & 15);
                }
            }
        }

        /** Changes since the last call, as (dx+16, dz+16, y, state) relative to the center origin. */
        void diff(List<int[]> out) throws Exception {
            for (int i = 0; i < 9; i++) {
                for (int s = 0; s < data[i].length; s++) {
                    LevelChunkSection section = chunks[i].getSection(s);
                    Object d = sectionData.get(section.getStates());
                    long[] r = ((net.minecraft.util.BitStorage) dataStorage.invoke(d)).getRaw();
                    if (d == data[i][s] && Arrays.equals(r, raw[i][s])) continue;
                    data[i][s] = d;
                    raw[i][s] = r.clone();
                    BlockState[] st = states[i][s];
                    for (int k = 0; k < 4096; k++) {
                        BlockState now = section.getBlockState(k & 15, k >> 8, (k >> 4) & 15);
                        if (now != st[k]) {
                            st[k] = now;
                            int dx = (i % 3 - 1) * 16 + (k & 15), dz = (i / 3 - 1) * 16 + ((k >> 4) & 15);
                            out.add(new int[] {dx + 16, dz + 16, minY() + s * 16 + (k >> 8), Block.BLOCK_STATE_REGISTRY.getId(now)});
                        }
                    }
                }
            }
        }
    }

    static BoundingBox writableArea(ChunkAccess chunk) {
        ChunkPos p = chunk.getPos();
        int x = p.getMinBlockX(), z = p.getMinBlockZ();
        var h = chunk.getHeightAccessorForGeneration();
        return new BoundingBox(x, h.getMinY() + 1, z, x + 15, h.getMaxY(), z + 15);
    }

    /** ChunkGenerator.applyBiomeDecoration, recording the changes of each placement. */
    void decorate(ProtoChunk chunk, HarnessRegion region, StructureManager sm, Shadow shadow, DataOutputStream log) throws Exception {
        ChunkPos cp = chunk.getPos();
        SectionPos sp = SectionPos.of(cp, region.getMinSectionY());
        BlockPos origin = sp.origin();
        Registry<Structure> structureRegistry = registries.lookupOrThrow(Registries.STRUCTURE);
        Map<Integer, List<Structure>> byStep = structureRegistry.stream().collect(Collectors.groupingBy(s -> s.step().ordinal()));
        WorldgenRandom rnd = new WorldgenRandom(new XoroshiroRandomSource(RandomSupport.generateUniqueSeed()));
        long decorationSeed = rnd.setDecorationSeed(region.getSeed(), origin.getX(), origin.getZ());
        Set<Holder<Biome>> present = new ObjectArraySet<>();
        ChunkPos.rangeClosed(sp.chunk(), 1).forEach(p -> {
            for (LevelChunkSection s : region.getChunk(p.x(), p.z()).getSections()) s.getBiomes().getAll(present::add);
        });
        present.retainAll(biomeSource.possibleBiomes());
        int stepCount = steps.size();
        FeaturePlacer placer = new FeaturePlacer(region, generator);
        int total = Math.max(GenerationStep.Decoration.values().length, stepCount);
        List<int[]> changes = new ArrayList<>();
        ByteArrayOutputStream body = new ByteArrayOutputStream();
        DataOutputStream out = new DataOutputStream(body);
        int invocations = 0;
        for (int step = 0; step < total; step++) {
            int j = 0;
            if (sm.shouldGenerateStructures()) {
                for (Structure s : byStep.getOrDefault(step, List.of())) {
                    rnd.setFeatureSeed(decorationSeed, j, step);
                    region.farReads = 0;
                    for (var start : sm.startsForStructure(sp.x(), sp.z(), s)) {
                        start.placeInChunk(region, sm, generator, rnd, writableArea(chunk), cp);
                    }
                    record(out, 1, step, j, region, shadow, changes);
                    invocations++;
                    j++;
                }
            }
            if (step < stepCount) {
                IntSet indices = new IntArraySet();
                FeatureSorter.StepFeatureData data = steps.get(step);
                for (Holder<Biome> b : present) {
                    List<HolderSet<PlacedFeature>> features = settingsGetter.apply(b).features();
                    if (step < features.size()) {
                        for (Holder<PlacedFeature> f : features.get(step)) indices.add(data.indexMapping().applyAsInt(f.value()));
                    }
                }
                int[] sorted = indices.toIntArray();
                Arrays.sort(sorted);
                for (int i : sorted) {
                    PlacedFeature f = data.features().get(i);
                    rnd.setFeatureSeed(decorationSeed, i, step);
                    region.farReads = 0;
                    placer.placeWithBiomeCheck(f, rnd, origin);
                    record(out, 0, step, i, region, shadow, changes);
                    invocations++;
                }
            }
        }
        out.flush();
        byte[] rec = body.toByteArray();
        ByteArrayOutputStream full = new ByteArrayOutputStream(rec.length + 4);
        DataOutputStream fo = new DataOutputStream(full);
        fo.writeInt(Integer.reverseBytes(invocations));
        fo.write(rec);
        fo.flush();
        byte[] z = deflate(full.toByteArray());
        log.writeInt(Integer.reverseBytes(cp.x()));
        log.writeInt(Integer.reverseBytes(cp.z()));
        log.writeInt(Integer.reverseBytes(z.length));
        log.write(z);
    }

    void record(DataOutputStream out, int kind, int step, int index, HarnessRegion region, Shadow shadow, List<int[]> changes) throws Exception {
        changes.clear();
        if (shadow != null) shadow.diff(changes);
        out.writeByte(kind);
        out.writeByte(step);
        out.writeShort(Short.reverseBytes((short) index));
        out.writeInt(Integer.reverseBytes((int) Math.min(region.farReads, Integer.MAX_VALUE)));
        out.writeInt(Integer.reverseBytes(changes.size()));
        for (int[] c : changes) {
            out.writeByte(c[0]);
            out.writeByte(c[1]);
            out.writeShort(Short.reverseBytes((short) c[2]));
            out.writeShort(Short.reverseBytes((short) c[3]));
        }
    }

    // ---- Regions ------------------------------------------------------------------------

    /** Chunks to decorate so every target chunk is final: the targets' 3x3 neighbourhoods
     *  closed under "lower-ranked chunks within distance 2 go first". */
    static List<long[]> decorationOrder(List<int[]> targets) {
        Set<Long> set = new HashSet<>();
        ArrayList<int[]> stack = new ArrayList<>();
        for (int[] t : targets) {
            for (int dx = -1; dx <= 1; dx++) for (int dz = -1; dz <= 1; dz++) {
                if (set.add(ChunkPos.pack(t[0] + dx, t[1] + dz))) stack.add(new int[] {t[0] + dx, t[1] + dz});
            }
        }
        while (!stack.isEmpty()) {
            int[] c = stack.removeLast();
            int r = rank(c[0], c[1]);
            for (int dx = -2; dx <= 2; dx++) for (int dz = -2; dz <= 2; dz++) {
                int x = c[0] + dx, z = c[1] + dz;
                if (rank(x, z) < r && set.add(ChunkPos.pack(x, z))) stack.add(new int[] {x, z});
            }
        }
        List<long[]> out = new ArrayList<>();
        for (long p : set) out.add(new long[] {rank(ChunkPos.getX(p), ChunkPos.getZ(p)), ChunkPos.getX(p), ChunkPos.getZ(p)});
        out.sort(Comparator.<long[]>comparingLong(a -> a[0]).thenComparingLong(a -> a[1]).thenComparingLong(a -> a[2]));
        return out;
    }

    static List<int[]> regionOrigins(long seed, int count) {
        List<int[]> out = new ArrayList<>();
        Random r = new Random(seed ^ 0x4B494C4E46454154L);
        for (int i = 0; i < count; i++) {
            if (i < FIXED_REGIONS.length) out.add(FIXED_REGIONS[i]);
            else if (i % 2 == 0) out.add(new int[] {r.nextInt(4000) - 2000, r.nextInt(4000) - 2000});
            else out.add(new int[] {r.nextInt(3_700_000) - 1_850_000, r.nextInt(3_700_000) - 1_850_000});
        }
        return out;
    }

    /** Chunks within `r` of any of `centers`. */
    static Set<Long> around(Iterable<Long> centers, int r) {
        Set<Long> out = new HashSet<>();
        for (long p : centers) {
            for (int dx = -r; dx <= r; dx++) for (int dz = -r; dz <= r; dz++) out.add(ChunkPos.pack(ChunkPos.getX(p) + dx, ChunkPos.getZ(p) + dz));
        }
        return out;
    }

    static <T> void await(List<Future<T>> futures) throws Exception {
        for (Future<T> f : futures) f.get();
    }

    /**
     * TERRAIN for the decorated chunks and their neighbours. With structures: STRUCTURE_STARTS
     * for every chunk within 8 of those, STRUCTURE_REFERENCES, and the beardifier in TERRAIN.
     */
    Map<Long, ProtoChunk> terrainFor(List<long[]> order, ExecutorService pool) throws Exception {
        List<Long> decorated = new ArrayList<>();
        for (long[] d : order) decorated.add(ChunkPos.pack((int) d[1], (int) d[2]));
        Set<Long> need = around(decorated, 1);
        Map<Long, ProtoChunk> chunks = new ConcurrentHashMap<>();
        if (!structures) {
            List<Future<ProtoChunk>> futures = new ArrayList<>();
            for (long p : need) futures.add(pool.submit(() -> {
                ProtoChunk c = terrain(ChunkPos.getX(p), ChunkPos.getZ(p));
                chunks.put(p, c);
                return c;
            }));
            await(futures);
            return chunks;
        }
        for (long p : around(need, 8)) chunks.put(p, new ProtoChunk(new ChunkPos(ChunkPos.getX(p), ChunkPos.getZ(p)), UpgradeData.EMPTY, level, containers, null));
        // One thread: template palettes cache lazily in plain HashMaps.
        for (ProtoChunk c : chunks.values()) {
            generator.createStructures(registries, level.getChunkSource().getGeneratorState(), level.structureManager(), c,
                server.getStructureTemplateManager(), level.dimension());
            c.setPersistedStatus(ChunkStatus.STRUCTURE_STARTS);
        }
        List<Future<Object>> futures = new ArrayList<>();
        for (long p : need) futures.add(pool.submit(() -> {
            ProtoChunk c = chunks.get(p);
            HarnessRegion region = region(c, chunks, ChunkStatus.STRUCTURE_REFERENCES);
            generator.createReferences(region, level.structureManager().forWorldGenRegion(region), c);
            c.setPersistedStatus(ChunkStatus.STRUCTURE_REFERENCES);
            return null;
        }));
        await(futures);
        futures.clear();
        for (long p : need) futures.add(pool.submit(() -> {
            ProtoChunk c = chunks.get(p);
            HarnessRegion region = region(c, chunks, ChunkStatus.TERRAIN);
            terrain(c, Beardifier.forStructuresInChunk(level.structureManager().forWorldGenRegion(region), c.getPos()));
            return null;
        }));
        await(futures);
        return chunks;
    }

    /** Structure starts (with their saved NBT), references and post-TERRAIN blocks. */
    void writeStructures(DataOutputStream w, Map<Long, ProtoChunk> chunks, List<long[]> order) throws Exception {
        var context = net.minecraft.world.level.levelgen.structure.pieces.StructurePieceSerializationContext.fromLevel(level);
        Registry<Structure> structureRegistry = registries.lookupOrThrow(Registries.STRUCTURE);
        List<Long> keys = new ArrayList<>(chunks.keySet());
        keys.sort(null);
        List<ProtoChunk> withStarts = new ArrayList<>();
        for (long k : keys) if (!chunks.get(k).getAllStarts().isEmpty()) withStarts.add(chunks.get(k));
        w.writeInt(Integer.reverseBytes(withStarts.size()));
        for (ProtoChunk c : withStarts) {
            w.writeInt(Integer.reverseBytes(c.getPos().x()));
            w.writeInt(Integer.reverseBytes(c.getPos().z()));
            w.writeInt(Integer.reverseBytes(c.getAllStarts().size()));
            for (var e : c.getAllStarts().entrySet()) {
                str(w, structureRegistry.getKey(e.getKey()).toString());
                ByteArrayOutputStream nbt = new ByteArrayOutputStream();
                net.minecraft.nbt.NbtIo.write(e.getValue().createTag(context, c.getPos()), new DataOutputStream(nbt));
                w.writeInt(Integer.reverseBytes(nbt.size()));
                w.write(nbt.toByteArray());
            }
        }
        List<ProtoChunk> withRefs = new ArrayList<>();
        for (long k : keys) if (!chunks.get(k).getAllReferences().isEmpty()) withRefs.add(chunks.get(k));
        w.writeInt(Integer.reverseBytes(withRefs.size()));
        for (ProtoChunk c : withRefs) {
            w.writeInt(Integer.reverseBytes(c.getPos().x()));
            w.writeInt(Integer.reverseBytes(c.getPos().z()));
            w.writeInt(Integer.reverseBytes(c.getAllReferences().size()));
            for (var e : c.getAllReferences().entrySet()) {
                str(w, structureRegistry.getKey(e.getKey()).toString());
                long[] refs = e.getValue().toLongArray();
                Arrays.sort(refs);
                w.writeInt(Integer.reverseBytes(refs.length));
                for (long r : refs) w.writeLong(Long.reverseBytes(r));
            }
        }
        w.writeInt(Integer.reverseBytes(order.size()));
        for (long[] d : order) {
            byte[] z = deflate(blocks(chunks.get(ChunkPos.pack((int) d[1], (int) d[2]))));
            w.writeInt(Integer.reverseBytes((int) d[1]));
            w.writeInt(Integer.reverseBytes((int) d[2]));
            w.writeInt(Integer.reverseBytes(z.length));
            w.write(z);
        }
    }

    void writeRegion(DataOutputStream w, List<int[]> targets, ExecutorService pool, boolean check) throws Exception {
        List<long[]> order = decorationOrder(targets);
        long t0 = System.nanoTime();
        Map<Long, ProtoChunk> chunks = terrainFor(order, pool);
        long t1 = System.nanoTime();
        w.writeInt(Integer.reverseBytes(targets.size()));
        for (int[] t : targets) {
            w.writeInt(Integer.reverseBytes(t[0]));
            w.writeInt(Integer.reverseBytes(t[1]));
        }
        if (structures) writeStructures(w, chunks, order);
        w.writeInt(Integer.reverseBytes(order.size()));
        for (long[] d : order) {
            ProtoChunk chunk = chunks.get(ChunkPos.pack((int) d[1], (int) d[2]));
            HarnessRegion region = region(chunk, chunks);
            Shadow shadow = new Shadow((int) d[1], (int) d[2], chunks);
            decorate(chunk, region, level.structureManager().forWorldGenRegion(region), shadow, w);
            chunk.setPersistedStatus(ChunkStatus.FEATURES);
        }
        long t2 = System.nanoTime();
        for (int[] t : targets) {
            ProtoChunk chunk = chunks.get(ChunkPos.pack(t[0], t[1]));
            byte[] blocks = blocks(chunk);
            byte[] z = deflate(blocks);
            w.writeInt(Integer.reverseBytes(t[0]));
            w.writeInt(Integer.reverseBytes(t[1]));
            w.writeInt(Integer.reverseBytes(z.length));
            w.write(z);
        }
        OUT.printf("  region at %d,%d: %d targets, %d decorated, %d terrain chunks; terrain %.1fs, features (with diffs) %.1fs%n",
            targets.get(0)[0], targets.get(0)[1], targets.size(), order.size(), chunks.size(), (t1 - t0) / 1e9, (t2 - t1) / 1e9);
        if (check) check(targets, order, chunks, pool);
    }

    /** Re-generates the region with vanilla's own applyBiomeDecoration and compares targets. */
    void check(List<int[]> targets, List<long[]> order, Map<Long, ProtoChunk> mine, ExecutorService pool) throws Exception {
        Map<Long, ProtoChunk> chunks = terrainFor(order, pool);
        long t0 = System.nanoTime();
        for (long[] d : order) {
            ProtoChunk chunk = chunks.get(ChunkPos.pack((int) d[1], (int) d[2]));
            HarnessRegion region = region(chunk, chunks);
            generator.applyBiomeDecoration(region, chunk, level.structureManager().forWorldGenRegion(region));
            chunk.setPersistedStatus(ChunkStatus.FEATURES);
        }
        long t1 = System.nanoTime();
        int bad = 0;
        for (int[] t : targets) {
            if (!Arrays.equals(blocks(chunks.get(ChunkPos.pack(t[0], t[1]))), blocks(mine.get(ChunkPos.pack(t[0], t[1]))))) bad++;
        }
        OUT.printf("  check: vanilla applyBiomeDecoration %.2f ms/chunk; %d of %d target chunks differ from the harness loop%n",
            (t1 - t0) / 1e6 / order.size(), bad, targets.size());
    }

    byte[] blocks(ProtoChunk chunk) {
        ByteBuffer buf = ByteBuffer.allocate(sections() * 4096 * 2).order(ByteOrder.LITTLE_ENDIAN);
        for (int s = 0; s < sections(); s++) {
            LevelChunkSection section = chunk.getSection(s);
            for (int i = 0; i < 4096; i++) {
                buf.putShort((short) Block.BLOCK_STATE_REGISTRY.getId(section.getBlockState(i & 15, i >> 8, (i >> 4) & 15)));
            }
        }
        return buf.array();
    }

    static byte[] deflate(byte[] data) {
        Deflater d = new Deflater(1);
        d.setInput(data);
        d.finish();
        ByteArrayOutputStream out = new ByteArrayOutputStream(Math.max(64, data.length / 4));
        byte[] tmp = new byte[1 << 16];
        while (!d.finished()) out.write(tmp, 0, d.deflate(tmp));
        d.end();
        return out.toByteArray();
    }

    static void str(DataOutputStream w, String s) throws IOException {
        byte[] v = s.getBytes(StandardCharsets.UTF_8);
        w.writeInt(Integer.reverseBytes(v.length));
        w.write(v);
    }

    void write(Path path, int regions, int size, boolean check) throws Exception {
        Registry<PlacedFeature> placed = registries.lookupOrThrow(Registries.PLACED_FEATURE);
        int threads = Math.max(1, Runtime.getRuntime().availableProcessors() - 2);
        ExecutorService pool = Executors.newFixedThreadPool(threads);
        try (OutputStream file = new BufferedOutputStream(Files.newOutputStream(path), 1 << 20)) {
            DataOutputStream w = new DataOutputStream(file);
            w.write("KWGF".getBytes(StandardCharsets.US_ASCII));
            w.writeInt(Integer.reverseBytes(1));
            w.writeLong(Long.reverseBytes(seed));
            w.writeInt(Integer.reverseBytes(structures ? 1 : 0));
            w.writeInt(Integer.reverseBytes(Block.BLOCK_STATE_REGISTRY.size()));
            w.writeInt(Integer.reverseBytes(minY()));
            w.writeInt(Integer.reverseBytes(level.getHeight()));
            w.writeInt(Integer.reverseBytes(steps.size()));
            for (int step = 0; step < steps.size(); step++) {
                List<PlacedFeature> fs = steps.get(step).features();
                w.writeInt(Integer.reverseBytes(fs.size()));
                for (int i = 0; i < fs.size(); i++) {
                    var key = placed.getResourceKey(fs.get(i));
                    str(w, key.map(k -> k.identifier().toString()).orElse("#" + step + ":" + i));
                }
            }
            List<int[]> origins = regionOrigins(seed, regions);
            w.writeInt(Integer.reverseBytes(origins.size()));
            long t0 = System.nanoTime();
            for (int i = 0; i < origins.size(); i++) {
                int[] o = origins.get(i);
                List<int[]> targets = new ArrayList<>();
                for (int dz = 0; dz < size; dz++) for (int dx = 0; dx < size; dx++) targets.add(new int[] {o[0] + dx, o[1] + dz});
                writeRegion(w, targets, pool, check && i == 0);
            }
            w.flush();
            OUT.printf("seed %d: %d regions in %.1fs%n", seed, origins.size(), (System.nanoTime() - t0) / 1e9);
        } finally {
            pool.shutdown();
        }
    }

    /** Single-threaded vanilla FEATURES rate with applyBiomeDecoration, over n decorated chunks. */
    void bench(int n) throws Exception {
        int size = Math.max(1, (int) Math.sqrt(n) - 2);
        List<int[]> targets = new ArrayList<>();
        for (int dz = 0; dz < size; dz++) for (int dx = 0; dx < size; dx++) targets.add(new int[] {5000 + dx, -7000 + dz});
        List<long[]> order = decorationOrder(targets);
        ExecutorService pool = Executors.newFixedThreadPool(Math.max(1, Runtime.getRuntime().availableProcessors() - 2));
        double best = Double.MAX_VALUE;
        for (int round = 0; round < 3; round++) {
            Map<Long, ProtoChunk> chunks = terrainFor(order, pool);
            long t0 = System.nanoTime();
            for (long[] d : order) {
                ProtoChunk chunk = chunks.get(ChunkPos.pack((int) d[1], (int) d[2]));
                HarnessRegion region = region(chunk, chunks);
                generator.applyBiomeDecoration(region, chunk, level.structureManager().forWorldGenRegion(region));
                chunk.setPersistedStatus(ChunkStatus.FEATURES);
            }
            best = Math.min(best, (System.nanoTime() - t0) / 1e6 / order.size());
        }
        pool.shutdown();
        OUT.printf("vanilla FEATURES (applyBiomeDecoration, %s structures): %.3f ms/chunk over %d chunks, best of 3%n",
            structures ? "with" : "without", best, order.size());
    }

    /**
     * heights_<seed>.bin: "KWGH", count, then per column x, z, getBaseHeight for WORLD_SURFACE_WG
     * and OCEAN_FLOOR_WG, and the base column (state ids from the lowest y) as i16 count + u16s.
     */
    void heights(Path path, int n) throws Exception {
        Random r = new Random(seed ^ 0x48454947L);
        var h = LevelHeightAccessor.create(minY(), level.getHeight());
        try (OutputStream file = new BufferedOutputStream(Files.newOutputStream(path), 1 << 20)) {
            DataOutputStream w = new DataOutputStream(file);
            w.write("KWGH".getBytes(StandardCharsets.US_ASCII));
            w.writeInt(Integer.reverseBytes(n));
            for (int i = 0; i < n; i++) {
                int x = i % 4 == 0 ? r.nextInt(60_000_000) - 30_000_000 : r.nextInt(8000) - 4000;
                int z = i % 4 == 0 ? r.nextInt(60_000_000) - 30_000_000 : r.nextInt(8000) - 4000;
                w.writeInt(Integer.reverseBytes(x));
                w.writeInt(Integer.reverseBytes(z));
                w.writeInt(Integer.reverseBytes(generator.getBaseHeight(x, z, net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE_WG, h, random)));
                w.writeInt(Integer.reverseBytes(generator.getBaseHeight(x, z, net.minecraft.world.level.levelgen.Heightmap.Types.OCEAN_FLOOR_WG, h, random)));
                var column = generator.getBaseColumn(x, z, h, random);
                w.writeShort(Short.reverseBytes((short) level.getHeight()));
                for (int y = minY(); y < minY() + level.getHeight(); y++) {
                    w.writeShort(Short.reverseBytes((short) Block.BLOCK_STATE_REGISTRY.getId(column.getBlock(y))));
                }
            }
            w.flush();
        }
        OUT.printf("seed %d: %d base heights%n", seed, n);
    }

    static MinecraftServer findServer() throws Exception {
        for (int attempt = 0; attempt < 600; attempt++) {
            for (Thread t : Thread.getAllStackTraces().keySet()) {
                if (!t.getName().equals("Server thread")) continue;
                Field holder = Thread.class.getDeclaredField("holder");
                holder.setAccessible(true);
                Object h = holder.get(t);
                Field task = h.getClass().getDeclaredField("task");
                task.setAccessible(true);
                Object r = task.get(h);
                for (Field f : r.getClass().getDeclaredFields()) {
                    f.setAccessible(true);
                    if (f.get(r) instanceof AtomicReference<?> ref && ref.get() instanceof MinecraftServer s) return s;
                }
            }
            Thread.sleep(100);
        }
        throw new IllegalStateException("no server thread");
    }

    public static void main(String[] args) throws Exception {
        Path out = Path.of(args[0]).toAbsolutePath();
        long seedArg = Long.parseLong(args[1]);
        int regions = FIXED_REGIONS.length + 4;
        int size = 4;
        int bench = 0;
        int heights = 0;
        boolean check = false;
        for (int i = 2; i < args.length; i++) {
            switch (args[i]) {
                case "--regions" -> regions = Integer.parseInt(args[++i]);
                case "--size" -> size = Integer.parseInt(args[++i]);
                case "--bench" -> bench = Integer.parseInt(args[++i]);
                case "--check" -> check = true;
                case "--heights" -> heights = Integer.parseInt(args[++i]);
                default -> throw new IllegalArgumentException(args[i]);
            }
        }
        Files.createDirectories(out);
        net.minecraft.server.Main.main(new String[] {"--nogui"});
        MinecraftServer server = findServer();
        while (!server.isReady()) Thread.sleep(100);
        FeatureVectors fv = new FeatureVectors(server);
        if (fv.seed != seedArg) throw new IllegalStateException("server seed " + fv.seed + " != " + seedArg);
        OUT.printf("server ready, seed %d, structures %b%n", fv.seed, fv.structures);
        try {
            if (regions > 0) fv.write(out.resolve("features_" + seedArg + (fv.structures ? "_s" : "") + ".bin"), regions, size, check);
            if (bench > 0) fv.bench(bench);
            if (heights > 0) fv.heights(out.resolve("heights_" + seedArg + ".bin"), heights);
        } catch (Throwable e) {
            e.printStackTrace(OUT);
            Runtime.getRuntime().halt(1);
        }
        Runtime.getRuntime().halt(0);
    }
}
