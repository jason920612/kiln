"""wp87: builds tools/wp87/FlatVectors.java from tools/wp85/PackVectors.java: the same vanilla harness for a dimension whose generator is
`minecraft:flat` (FlatLevelSource: the layers as terrain, no carvers, no surface rules, the biome's features when `features` is set, the
structure sets of `structure_overrides`). Run: python3 tools/wp87/make_flat_vectors.py
"""
from pathlib import Path

HERE = Path(__file__).resolve().parent
src = (HERE.parent / "wp85" / "PackVectors.java").read_text(encoding="utf-8")


def rep(a, b, count=1):
    global src
    assert src.count(a) == count, (a, src.count(a))
    src = src.replace(a, b)


rep("public class PackVectors {", "public class FlatVectors {")
rep("    PackVectors(MinecraftServer server, String dimension) throws Exception {", "    FlatVectors(MinecraftServer server, String dimension) throws Exception {")
rep("PackVectors fv = new PackVectors(", "FlatVectors fv = new FlatVectors(")
rep("    final NoiseBasedChunkGenerator generator;", "    final net.minecraft.world.level.levelgen.FlatLevelSource generator;")
rep("    final NoiseGeneratorSettings settings;", "    final NoiseGeneratorSettings settings = null;")
rep("    final net.minecraft.world.level.levelgen.NoiseSettings genSettings;", "    final net.minecraft.world.level.levelgen.NoiseSettings genSettings = null;")
rep("    final Aquifer.FluidPicker fluidPicker;", "    final Aquifer.FluidPicker fluidPicker = null;")
rep("    final Method doFill, buildSurface, generateCarvers;", "    final Method doFill = null, buildSurface = null, generateCarvers = null;")
rep("        generator = (NoiseBasedChunkGenerator) level.getChunkSource().getGenerator();", "        generator = (net.minecraft.world.level.levelgen.FlatLevelSource) level.getChunkSource().getGenerator();")
rep("        settings = generator.generatorSettings().value();\n", "")
rep("        genSettings = settings.noiseSettings().clampToHeightAccessor(level);\n", "")
a = src.index("        Field picker = NoiseBasedChunkGenerator.class.getDeclaredField(\"globalFluidPicker\");")
b = src.index("        Field fps = ")
src = src[:a] + src[b:]
a = src.index("        DensityVolume volume = new DensityVolume(")
b = src.index("        chunk.setPersistedStatus(ChunkStatus.TERRAIN);")
src = src[:a] + "        generator.buildTerrain(chunk, Blender.empty(), random, level.structureManager(), biomeManager, null, java.util.Set.of()).get();\n" + src[b:]
(HERE / "FlatVectors.java").write_text(src, encoding="utf-8")
print("wrote", HERE / "FlatVectors.java")
