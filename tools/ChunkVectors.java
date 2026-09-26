// Runs vanilla 26.3's overworld chunk generation in-process (BIOMES, then TERRAIN's fill,
// surface and carver steps, without structures or features) and dumps the results for
// kiln-worldgen's chunk parity test.
//
// Per seed it writes chunks_<seed>.bin:
//   header   "KWGC", version, seed, min y, height, block state count, biome names,
//            the overworld parameter list (quantized), chunk count
//   chunks   x, z, then a deflated blob: one biome index byte per quart (sections bottom to
//            top, each 4x4x4 in y, z, x order) and, for each of the three steps (fill,
//            surface, carvers), 16*384*16 block state ids as u16 in Kiln's section order
//            (sections bottom to top, index y << 8 | z << 4 | x).
//
// Canonical order: the climate R-tree keeps a thread-local "last result" that breaks ties,
// so vanilla's own biomes depend on which chunks a worker thread generated before. Here every
// chunk's biome fill starts from an empty last result (Kiln does the same), and the surface
// step reads neighbour biomes from the same canonical fills.
//
// usage: java -cp <server jar + libraries> tools/ChunkVectors.java <out dir> [--regions N]
//        [--bench N] [seed...]   (default seeds 0 1 12345 -4172144997902289642; "random" works)

import java.io.BufferedOutputStream;
import java.io.ByteArrayOutputStream;
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
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.function.Supplier;
import java.util.zip.Deflater;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderGetter;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.biome.BiomeResolver;
import net.minecraft.world.level.biome.Climate;
import net.minecraft.world.level.biome.MultiNoiseBiomeSource;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterLists;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.Aquifer;
import net.minecraft.world.level.levelgen.Beardifier;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseChunk;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;
import net.minecraft.world.level.levelgen.material.rule.MaterialRule;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

public class ChunkVectors {
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);

    // Fixed 8x8-chunk regions (origin, moderate and far coordinates, the 2^24 noise wrap and
    // the world border); the rest of each seed's regions are drawn from a seeded Random.
    static final int[][] FIXED_REGIONS = {
        {0, 0}, {-8, -8}, {100, -300}, {-2000, 1500}, {10000, 10000}, {-62500, 62500},
        {1048572, -1048580}, {1874930, 1874930}, {-1874938, -1874938}, {1874930, -1874938},
        {-1874938, 1874930}, {123456, -654321}, {-777777, 333333},
    };
    static final int TOTAL_REGIONS = 40;

    final RegistryAccess.Frozen registries;
    final NoiseGeneratorSettings settings;
    final Holder<NoiseGeneratorSettings> settingsHolder;
    final HolderGetter<NormalNoise> noises;
    final MultiNoiseBiomeSource biomeSource;
    final NoiseBasedChunkGenerator generator;
    final PalettedContainerFactory containers;
    final LevelHeightAccessor height;
    final List<String> biomeNames = new ArrayList<>();
    final Map<Holder<Biome>, Integer> biomeIndex = new HashMap<>();
    final ThreadLocal<?> lastResult;
    final Aquifer.FluidPicker fluidPicker;
    final Method doFill, buildSurface, generateCarvers;

    @SuppressWarnings("unchecked")
    ChunkVectors() throws Exception {
        registries = loadWorldgen();
        Registry<NoiseGeneratorSettings> settingsRegistry = registries.lookupOrThrow(Registries.NOISE_SETTINGS);
        settingsHolder = settingsRegistry.getOrThrow(NoiseGeneratorSettings.OVERWORLD);
        settings = settingsHolder.value();
        noises = registries.lookupOrThrow(Registries.NOISE);
        Holder<MultiNoiseBiomeSourceParameterList> preset = registries
            .lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST)
            .getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD);
        biomeSource = MultiNoiseBiomeSource.createFromPreset(preset);
        generator = new NoiseBasedChunkGenerator(biomeSource, settingsHolder);
        containers = PalettedContainerFactory.create(registries);
        height = LevelHeightAccessor.create(settings.noiseSettings().minY(), settings.noiseSettings().height());
        for (var ref : registries.lookupOrThrow(Registries.BIOME).listElements().toList()) {
            biomeIndex.put(ref, biomeNames.size());
            biomeNames.add(ref.key().identifier().toString());
        }

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
        doFill = NoiseBasedChunkGenerator.class.getDeclaredMethod("doFill", NoiseChunk.class, net.minecraft.world.level.chunk.ChunkAccess.class);
        buildSurface = NoiseBasedChunkGenerator.class.getDeclaredMethod("buildSurface", net.minecraft.world.level.chunk.ChunkAccess.class,
            NoiseChunk.class, RandomState.class, BiomeManager.class, Set.class, MaterialRule.class);
        generateCarvers = NoiseBasedChunkGenerator.class.getDeclaredMethod("generateCarvers", net.minecraft.world.level.chunk.ChunkAccess.class,
            Blender.class, NoiseChunk.class, RandomState.class, BiomeManager.class, net.minecraft.server.level.WorldGenRegion.class,
            MaterialRule.class);
        for (Method m : new Method[] {doFill, buildSurface, generateCarvers}) m.setAccessible(true);
    }

    static RegistryAccess.Frozen loadWorldgen() {
        var repository = ServerPacksSource.createVanillaTrustedRepository();
        repository.reload();
        repository.setSelected(List.of("vanilla"));
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repository.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var pendingTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var lookups = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), pendingTags);
        return RegistryDataLoader.load(resources, lookups, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
    }

    /** Per-seed state: the random state and every chunk's canonical biome fill. */
    final class Seed {
        final long seed;
        final RandomState random;
        final long zoomSeed;
        final Map<Long, Holder<Biome>[]> biomes = new ConcurrentHashMap<>();
        final BiomeManager biomeManager;

        Seed(long seed) {
            this.seed = seed;
            random = RandomState.create(noises, seed, settings);
            zoomSeed = BiomeManager.obfuscateSeed(seed);
            biomeManager = new BiomeManager(this::storedBiome, zoomSeed);
        }

        int sections() {
            return height.getSectionsCount();
        }

        Holder<Biome>[] chunkBiomes(int cx, int cz) {
            return biomes.computeIfAbsent(ChunkPos.pack(cx, cz), k -> computeBiomes(cx, cz));
        }

        /** What ChunkGenerator.doCreateBiomes stores for a chunk, from an empty R-tree history. */
        @SuppressWarnings("unchecked")
        Holder<Biome>[] computeBiomes(int cx, int cz) {
            lastResult.remove();
            SamplerContext ctx = SamplerContext.builder().enableCaches().build();
            int qx = cx * 4, qz = cz * 4, qy = height.getMinY() >> 2;
            BiomeResolver resolver = biomeSource.createResolverForChunk(random.createClimateSampler(ctx), qx, qy, qz, 4,
                height.getHeight() >> 2, 4);
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
            int s = Math.clamp((qy - (height.getMinY() >> 2)) >> 2, 0, sections() - 1);
            int ly = Math.clamp(qy - (height.getMinY() >> 2) - s * 4, 0, 3);
            return chunkBiomes(qx >> 2, qz >> 2)[s * 64 + (ly << 4 | (qz & 3) << 2 | (qx & 3))];
        }

        ProtoChunk newChunk(int cx, int cz) {
            ProtoChunk chunk = new ProtoChunk(new ChunkPos(cx, cz), UpgradeData.EMPTY, height, containers, null);
            Holder<Biome>[] b = chunkBiomes(cx, cz);
            int minQy = height.getMinY() >> 2;
            chunk.fillBiomesFromNoise((x, y, z) -> {
                int s = (y - minQy) >> 2;
                return b[s * 64 + (((y - minQy) & 3) << 4 | (z & 3) << 2 | (x & 3))];
            });
            chunk.setPersistedStatus(ChunkStatus.BIOMES);
            return chunk;
        }

        NoiseChunk noiseChunk(ProtoChunk chunk) {
            ChunkPos pos = chunk.getPos();
            DensityVolume volume = new DensityVolume(16, height.getHeight(), 16, pos.getMinBlockX(), height.getMinY(), pos.getMinBlockZ());
            return new NoiseChunk(random, Beardifier.EMPTY, settings, fluidPicker, Blender.empty(), volume);
        }

        void fill(ProtoChunk chunk, NoiseChunk noise) throws Exception {
            doFill.invoke(generator, noise, chunk);
        }

        void surface(ProtoChunk chunk, NoiseChunk noise) throws Exception {
            buildSurface.invoke(generator, chunk, noise, random, biomeManager, biomeSource.possibleBiomes(), settings.materialRule().value());
        }

        void carvers(ProtoChunk chunk, NoiseChunk noise) throws Exception {
            generateCarvers.invoke(generator, chunk, Blender.empty(), noise, random, biomeManager, null, settings.materialRule().value());
        }

        /** Generates one chunk and returns its deflated record. */
        byte[] record(int cx, int cz) throws Exception {
            ByteBuffer buf = ByteBuffer.allocate(sections() * 64 + 3 * sections() * 4096 * 2).order(ByteOrder.LITTLE_ENDIAN);
            for (Holder<Biome> h : chunkBiomes(cx, cz)) buf.put((byte) (int) biomeIndex.get(h));
            ProtoChunk chunk = newChunk(cx, cz);
            try (NoiseChunk noise = noiseChunk(chunk)) {
                fill(chunk, noise);
                putBlocks(buf, chunk);
                surface(chunk, noise);
                putBlocks(buf, chunk);
                carvers(chunk, noise);
                putBlocks(buf, chunk);
            }
            Deflater d = new Deflater(1);
            d.setInput(buf.array());
            d.finish();
            ByteArrayOutputStream out = new ByteArrayOutputStream(1 << 16);
            byte[] tmp = new byte[1 << 16];
            while (!d.finished()) out.write(tmp, 0, d.deflate(tmp));
            d.end();
            return out.toByteArray();
        }

        void putBlocks(ByteBuffer buf, ProtoChunk chunk) {
            for (int s = 0; s < sections(); s++) {
                LevelChunkSection section = chunk.getSection(s);
                for (int i = 0; i < 4096; i++) {
                    buf.putShort((short) Block.BLOCK_STATE_REGISTRY.getId(section.getBlockState(i & 15, i >> 8, (i >> 4) & 15)));
                }
            }
        }
    }

    static List<int[]> chunkList(long seed, int regions) {
        List<int[]> out = new ArrayList<>();
        Random r = new Random(seed ^ 0x4B494C4E43484EL);
        for (int i = 0; i < regions; i++) {
            int[] origin = i < FIXED_REGIONS.length
                ? FIXED_REGIONS[i]
                : new int[] {r.nextInt(3_749_000) - 1_874_500, r.nextInt(3_749_000) - 1_874_500};
            if (i >= FIXED_REGIONS.length && i % 3 == 0) {
                origin = new int[] {r.nextInt(4000) - 2000, r.nextInt(4000) - 2000};
            }
            for (int dz = 0; dz < 8; dz++) for (int dx = 0; dx < 8; dx++) out.add(new int[] {origin[0] + dx, origin[1] + dz});
        }
        return out;
    }

    void write(Path path, long seedValue, int regions) throws Exception {
        Seed seed = new Seed(seedValue);
        List<int[]> chunks = chunkList(seedValue, regions);
        int threads = Math.max(1, Runtime.getRuntime().availableProcessors() - 2);
        ExecutorService pool = Executors.newFixedThreadPool(threads);
        List<Future<byte[]>> records = new ArrayList<>();
        long t0 = System.nanoTime();
        for (int[] c : chunks) records.add(pool.submit(() -> seed.record(c[0], c[1])));
        try (Writer w = new Writer(path)) {
            w.bytes("KWGC".getBytes(StandardCharsets.US_ASCII));
            w.i32(1);
            w.i64(seedValue);
            w.i32(height.getMinY());
            w.i32(height.getHeight());
            w.i32(Block.BLOCK_STATE_REGISTRY.size());
            w.i32(biomeNames.size());
            for (String n : biomeNames) w.str(n);
            writeParameters(w);
            w.i32(chunks.size());
            for (int i = 0; i < chunks.size(); i++) {
                byte[] rec = records.get(i).get();
                w.i32(chunks.get(i)[0]);
                w.i32(chunks.get(i)[1]);
                w.i32(rec.length);
                w.bytes(rec);
            }
        }
        pool.shutdown();
        OUT.printf("seed %d: %d chunks in %.1fs (%d threads)%n", seedValue, chunks.size(), (System.nanoTime() - t0) / 1e9, threads);
    }

    /** The preset's parameter list as vanilla holds it (quantized), to check Kiln's loader. */
    @SuppressWarnings("unchecked")
    void writeParameters(Writer w) throws Exception {
        Method parameters = MultiNoiseBiomeSource.class.getDeclaredMethod("parameters");
        parameters.setAccessible(true);
        Climate.ParameterList<Holder<Biome>> list = (Climate.ParameterList<Holder<Biome>>) parameters.invoke(biomeSource);
        w.i32(list.values().size());
        for (var pair : list.values()) {
            w.i32(biomeIndex.get(pair.getSecond()));
            Climate.ParameterPoint p = pair.getFirst();
            for (Climate.Parameter q : List.of(p.temperature(), p.humidity(), p.continentalness(), p.erosion(), p.depth(), p.weirdness())) {
                w.i64(q.min());
                w.i64(q.max());
            }
            w.i64(p.offset());
            w.i64(p.offset());
        }
    }

    /** Single-threaded vanilla rates per step, best of three rounds over `n` chunks. */
    void bench(long seedValue, int n) throws Exception {
        Seed seed = new Seed(seedValue);
        List<int[]> chunks = chunkList(seedValue, (n + 63) / 64).subList(0, n);
        double[] best = new double[4];
        for (int round = 0; round < 3; round++) {
            long[] t = new long[4];
            long a = System.nanoTime();
            for (int[] c : chunks) seed.computeBiomes(c[0], c[1]);
            t[0] = System.nanoTime() - a;
            // Neighbour biomes are already stored when vanilla builds a chunk's surface.
            for (int[] c : chunks) {
                for (int dx = -1; dx <= 1; dx++) for (int dz = -1; dz <= 1; dz++) seed.chunkBiomes(c[0] + dx, c[1] + dz);
            }
            for (int[] c : chunks) {
                ProtoChunk chunk = seed.newChunk(c[0], c[1]);
                long b = System.nanoTime();
                try (NoiseChunk noise = seed.noiseChunk(chunk)) {
                    seed.fill(chunk, noise);
                    long d = System.nanoTime();
                    seed.surface(chunk, noise);
                    long e = System.nanoTime();
                    seed.carvers(chunk, noise);
                    long f = System.nanoTime();
                    t[1] += d - b;
                    t[2] += e - d;
                    t[3] += f - e;
                }
            }
            for (int i = 0; i < 4; i++) best[i] = round == 0 ? t[i] : Math.min(best[i], t[i]);
        }
        String[] names = {"biomes", "fill (incl. aquifer)", "surface", "carvers"};
        double total = 0;
        for (int i = 0; i < 4; i++) {
            OUT.printf("vanilla %-22s %8.3f ms/chunk%n", names[i], best[i] / 1e6 / n);
            total += best[i];
        }
        OUT.printf("vanilla total                  %8.3f ms/chunk = %.1f chunks/s single-threaded%n", total / 1e6 / n, n / (total / 1e9));
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Path out = Path.of(args[0]);
        Files.createDirectories(out);
        int regions = TOTAL_REGIONS;
        int bench = 0;
        List<Long> seeds = new ArrayList<>();
        for (int i = 1; i < args.length; i++) {
            switch (args[i]) {
                case "--regions" -> regions = Integer.parseInt(args[++i]);
                case "--bench" -> bench = Integer.parseInt(args[++i]);
                case "random" -> seeds.add(new Random().nextLong());
                default -> seeds.add(Long.parseLong(args[i]));
            }
        }
        if (seeds.isEmpty() && bench == 0) seeds.addAll(List.of(0L, 1L, 12345L, -4172144997902289642L));
        ChunkVectors cv = new ChunkVectors();
        for (long seed : seeds) cv.write(out.resolve("chunks_" + seed + ".bin"), seed, regions);
        if (bench > 0) cv.bench(seeds.isEmpty() ? 0 : seeds.get(0), bench);
    }

    static final class Writer implements AutoCloseable {
        final OutputStream out;
        final ByteBuffer b = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN);

        Writer(Path path) throws IOException {
            out = new BufferedOutputStream(Files.newOutputStream(path), 1 << 20);
        }

        void bytes(byte[] v) throws IOException {
            out.write(v);
        }

        void i32(int v) throws IOException {
            b.clear();
            b.putInt(v);
            out.write(b.array(), 0, 4);
        }

        void i64(long v) throws IOException {
            b.clear();
            b.putLong(v);
            out.write(b.array(), 0, 8);
        }

        void str(String s) throws IOException {
            byte[] v = s.getBytes(StandardCharsets.UTF_8);
            i32(v.length);
            out.write(v);
        }

        @Override
        public void close() throws IOException {
            out.close();
        }
    }
}
