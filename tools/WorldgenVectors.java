// Dumps vanilla 26.3 density function and noise values for kiln-worldgen's parity tests.
//
// Loads the worldgen registries from the vanilla pack inside the server jar (the way
// WorldLoader does), builds the overworld RandomState per seed and writes, as raw f32 bits:
//   router_<seed>.bin     every noise router output (and aquifer function) on the cell-corner
//                         grid (x, z step 4, y step 8, y -64..320) of 832 chunks in 13 regions,
//                         in three modes: P = sampleValue per position (uncached context),
//                         V = sampleVolume of the chunk's corner grid (uncached context),
//                         C = sampleVolume with a fresh caching context per chunk and output.
//   functions_<seed>.bin  every overworld density function registry entry, same modes, 4 chunks.
//   noise_<seed>.bin      every noise instance: get(x,y,z), get(x,z) and addToVolume.
//
// usage: java -cp <server jar + libraries> tools/WorldgenVectors.java <out dir> [seed or "random"...]
//        (default seeds: 0 1 12345 -4172144997902289642 and one random seed)

import java.io.BufferedOutputStream;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.TreeMap;
import java.util.stream.IntStream;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.NoiseRouter;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.densityfunction.DensityBuffer;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunction;
import net.minecraft.world.level.levelgen.densityfunction.DensitySampler;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;
import net.minecraft.world.level.levelgen.synth.Noise;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

public class WorldgenVectors {
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);
    static final int CORNERS = 5 * 49 * 5;

    // Region origins in chunks; each region is 8x8 chunks. Covers the origin, moderate and far
    // coordinates, the 2^24 noise wrap boundary and both signs of the +-29,999,000 border.
    static final int[][] REGIONS = {
        {0, 0}, {-8, -8}, {100, -300}, {-2000, 1500}, {10000, 10000}, {-62500, 62500},
        {1048572, -1048580}, {1874930, 1874930}, {-1874938, -1874938}, {1874930, -1874938},
        {-1874938, 1874930}, {123456, -654321}, {-777777, 333333},
    };

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        Path out = Path.of(args[0]);
        Files.createDirectories(out);
        List<Long> seeds = new ArrayList<>();
        for (int i = 1; i < args.length; i++) seeds.add(args[i].equals("random") ? new Random().nextLong() : Long.parseLong(args[i]));
        if (seeds.isEmpty()) {
            seeds.addAll(List.of(0L, 1L, 12345L, -4172144997902289642L, new Random().nextLong()));
        }

        RegistryAccess.Frozen worldgen = loadWorldgen();
        Registry<NoiseGeneratorSettings> settingsRegistry = worldgen.lookupOrThrow(Registries.NOISE_SETTINGS);
        Registry<NormalNoise> noises = worldgen.lookupOrThrow(Registries.NOISE);
        Registry<DensityFunction> functions = worldgen.lookupOrThrow(Registries.DENSITY_FUNCTION);
        NoiseGeneratorSettings settings = settingsRegistry.getValueOrThrow(NoiseGeneratorSettings.OVERWORLD);

        List<int[]> routerChunks = new ArrayList<>();
        for (int[] r : REGIONS) {
            for (int dz = 0; dz < 8; dz++) for (int dx = 0; dx < 8; dx++) routerChunks.add(new int[] {r[0] + dx, r[1] + dz});
        }
        List<int[]> functionChunks = List.of(new int[] {0, 0}, new int[] {-3, 7}, new int[] {1874937, -1874938}, new int[] {-62500, 1048576});

        Map<String, DensityFunction> routerOutputs = routerOutputs(settings);
        Map<String, DensityFunction> overworldFunctions = new TreeMap<>();
        for (Identifier id : functions.keySet()) {
            String name = id.toString();
            if (name.startsWith("minecraft:overworld/") || name.startsWith("minecraft:shift_")) {
                overworldFunctions.put(name, functions.getValue(id));
            }
        }

        for (long seed : seeds) {
            RandomState rs = RandomState.create(noises, seed, settings);
            long t0 = System.nanoTime();
            writeFunctions(out.resolve("router_" + seed + ".bin"), seed, rs, routerOutputs, routerChunks);
            long t1 = System.nanoTime();
            writeFunctions(out.resolve("functions_" + seed + ".bin"), seed, rs, overworldFunctions, functionChunks);
            writeNoises(out.resolve("noise_" + seed + ".bin"), seed, rs, noises);
            OUT.printf("seed %d: router %.1fs, total %.1fs%n", seed, (t1 - t0) / 1e9, (System.nanoTime() - t0) / 1e9);
        }
        benchmark(RandomState.create(noises, 0, settings), settings.noiseRouter().finalDensity(), "final_density");
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

    static Map<String, DensityFunction> routerOutputs(NoiseGeneratorSettings settings) {
        NoiseRouter r = settings.noiseRouter();
        Map<String, DensityFunction> m = new java.util.LinkedHashMap<>();
        m.put("temperature", r.temperature());
        m.put("vegetation", r.vegetation());
        m.put("continents", r.continents());
        m.put("erosion", r.erosion());
        m.put("depth", r.depth());
        m.put("ridges", r.ridges());
        m.put("chunk_surface_level", r.chunkSurfaceLevel());
        m.put("final_density", r.finalDensity());
        settings.aquifers().ifPresent(a -> {
            m.put("aquifers/barrier", a.barrierNoise());
            m.put("aquifers/fluid_level_floodedness", a.fluidLevelFloodednessNoise());
            m.put("aquifers/fluid_level_spread", a.fluidLevelSpreadNoise());
            m.put("aquifers/lava", a.lavaNoise());
            m.put("aquifers/exclusion", a.exclusion());
            m.put("aquifers/surface_level", a.surfaceLevel());
        });
        return m;
    }

    static DensityVolume cornerVolume(int[] chunk) {
        return new DensityVolume(5, 49, 5, chunk[0] * 16, -64, chunk[1] * 16, 4, 8, 4);
    }

    static void writeFunctions(Path path, long seed, RandomState rs, Map<String, DensityFunction> outputs, List<int[]> chunks)
            throws IOException {
        try (Writer w = new Writer(path)) {
            w.bytes("KWGV".getBytes(StandardCharsets.US_ASCII));
            w.i32(1);
            w.i64(seed);
            w.i32(outputs.size());
            for (String name : outputs.keySet()) w.str(name);
            w.i32(chunks.size());
            for (int[] c : chunks) {
                w.i32(c[0]);
                w.i32(c[1]);
            }
            w.i32(3);
            for (char mode : new char[] {'P', 'V', 'C'}) {
                w.i32(mode);
                for (DensityFunction f : outputs.values()) {
                    DensitySampler sampler = rs.getSampler(f);
                    float[] values = new float[chunks.size() * CORNERS];
                    IntStream.range(0, chunks.size()).parallel().forEach(ci -> {
                        DensityVolume v = cornerVolume(chunks.get(ci));
                        float[] chunk = sample(sampler, v, mode);
                        System.arraycopy(chunk, 0, values, ci * CORNERS, CORNERS);
                    });
                    for (float x : values) w.i32(Float.floatToRawIntBits(x));
                }
            }
        }
    }

    static float[] sample(DensitySampler sampler, DensityVolume v, char mode) {
        float[] out = new float[v.size()];
        switch (mode) {
            case 'P' -> {
                int i = 0;
                for (int z = 0; z < v.sizeZ(); z++) {
                    for (int x = 0; x < v.sizeX(); x++) {
                        for (int y = 0; y < v.sizeY(); y++) {
                            out[i++] = sampler.sampleValue(SamplerContext.EMPTY_UNCACHED, v.blockX(x), v.blockY(y), v.blockZ(z));
                        }
                    }
                }
            }
            case 'V', 'C' -> {
                SamplerContext ctx = mode == 'V' ? SamplerContext.EMPTY_UNCACHED : SamplerContext.builder().enableCaches().build();
                DensityBuffer buf = DensityBuffer.createUnpooled(v.size());
                sampler.sampleVolume(ctx, buf, v);
                for (int i = 0; i < out.length; i++) out[i] = buf.get(i);
            }
            default -> throw new IllegalArgumentException();
        }
        return out;
    }

    static void writeNoises(Path path, long seed, RandomState rs, Registry<NormalNoise> noises) throws IOException {
        Random r = new Random(0x4B494C4EL);
        int n3 = 4096, n2 = 1024;
        double[][] p3 = new double[n3][];
        for (int i = 0; i < n3; i++) p3[i] = new double[] {coordinate(r, i), coordinate(r, i + 1), coordinate(r, i + 2)};
        double[][] p2 = new double[n2][];
        for (int i = 0; i < n2; i++) p2[i] = new double[] {coordinate(r, i), coordinate(r, i + 3)};
        DensityVolume[] volumes = {
            new DensityVolume(5, 49, 5, 0, -64, 0, 4, 8, 4),
            new DensityVolume(3, 7, 2, -29999008, -64, 29998992, 4, 8, 4),
            new DensityVolume(4, 3, 4, 16777200, 0, -16777232, 1, 1, 1),
        };
        double[][] scales = {{1.0, 1.0}, {0.25, 0.0}, {0.25, 0.25}, {2.0, 0.6666666666666666}, {1500.0, 0.0}};
        try (Writer w = new Writer(path)) {
            w.bytes("KWGN".getBytes(StandardCharsets.US_ASCII));
            w.i32(1);
            w.i64(seed);
            List<ResourceKey<NormalNoise>> keys = new ArrayList<>(noises.registryKeySet());
            keys.sort((a, b) -> a.identifier().toString().compareTo(b.identifier().toString()));
            w.i32(keys.size());
            for (ResourceKey<NormalNoise> key : keys) {
                Noise noise = rs.getOrCreateNoise(key);
                w.str(key.identifier().toString());
                w.i32(n3);
                for (double[] p : p3) {
                    w.f64(p[0]);
                    w.f64(p[1]);
                    w.f64(p[2]);
                    w.i32(Float.floatToRawIntBits(noise.get(p[0], p[1], p[2])));
                }
                w.i32(n2);
                for (double[] p : p2) {
                    w.f64(p[0]);
                    w.f64(p[1]);
                    w.i32(Float.floatToRawIntBits(noise.get(p[0], p[1])));
                }
                w.i32(volumes.length * scales.length);
                for (DensityVolume v : volumes) {
                    for (double[] s : scales) {
                        for (int x : new int[] {v.sizeX(), v.sizeY(), v.sizeZ(), v.minBlockX(), v.minBlockY(), v.minBlockZ(),
                                v.stepBlockX(), v.stepBlockY(), v.stepBlockZ()}) {
                            w.i32(x);
                        }
                        w.f64(s[0]);
                        w.f64(s[1]);
                        float amp = 0.75f;
                        w.i32(Float.floatToRawIntBits(amp));
                        DensityBuffer buf = DensityBuffer.createUnpooled(v.size());
                        buf.fill(0.25f);
                        noise.addToVolume(buf, v, s[0], s[1], amp);
                        for (int i = 0; i < v.size(); i++) w.i32(Float.floatToRawIntBits(buf.get(i)));
                    }
                }
            }
        }
    }

    /** Mixed magnitudes: lattice points, halves, the wrap boundary and random reals up to 1e9. */
    static double coordinate(Random r, int i) {
        return switch (i % 8) {
            case 0 -> r.nextInt(2001) - 1000;
            case 1 -> (r.nextInt(4001) - 2000) * 0.5;
            case 2 -> 16777216.0 * (r.nextBoolean() ? 1 : -1) + (r.nextDouble() - 0.5) * 4;
            case 3 -> (r.nextDouble() * 2 - 1) * 1e9;
            case 4 -> (r.nextDouble() * 2 - 1) * 3e7;
            default -> (r.nextDouble() * 2 - 1) * Math.pow(10, r.nextInt(7));
        };
    }

    static void benchmark(RandomState rs, DensityFunction f, String name) {
        DensitySampler sampler = rs.getSampler(f);
        DensityBuffer buf = DensityBuffer.createUnpooled(CORNERS);
        long positions = 0;
        long start = System.nanoTime();
        for (int round = 0; round < 3; round++) {
            positions = 0;
            start = System.nanoTime();
            for (int cx = 0; cx < 32; cx++) {
                for (int cz = 0; cz < 32; cz++) {
                    sampler.sampleVolume(SamplerContext.EMPTY_UNCACHED, buf, cornerVolume(new int[] {cx, cz}));
                    positions += CORNERS;
                }
            }
        }
        double secs = (System.nanoTime() - start) / 1e9;
        OUT.printf("java %s volume: %.0f positions/s single-threaded%n", name, positions / secs);
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

        void f64(double v) throws IOException {
            i64(Double.doubleToRawLongBits(v));
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
