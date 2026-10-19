"""wp87: builds tools/wp87/OrderVectors.java from tools/wp85/PackVectors.java: the same vanilla harness (an in-process vanilla server as the
generation context, the chunks driven through BIOMES, STRUCTURE_STARTS..TERRAIN and FEATURES, the final blocks / biomes / structure starts
of target windows dumped) but the decoration order is the harness' to choose: -Dwp87.rank=r0,...,r8 replaces Kiln's canonical class
ranks (a chunk is decorated after every chunk within distance 2 that has a lower rank; any permutation of the nine classes is an order the
real vanilla server can also take, it decides by thread timing). Run: python3 tools/wp87/make_order_vectors.py
"""
from pathlib import Path

HERE = Path(__file__).resolve().parent
src = (HERE.parent / "wp85" / "PackVectors.java").read_text(encoding="utf-8")


def rep(a, b, count=1):
    global src
    assert src.count(a) == count, (a, src.count(a))
    src = src.replace(a, b)


rep("public class PackVectors {", "public class OrderVectors {")
rep("    PackVectors(MinecraftServer server, String dimension) throws Exception {", "    OrderVectors(MinecraftServer server, String dimension) throws Exception {")
rep("    static final int[] RANK = {0, 1, 2, 7, 6, 3, 8, 5, 4};",
    "    static final int[] RANK = java.util.Arrays.stream(System.getProperty(\"wp87.rank\", \"0,1,2,7,6,3,8,5,4\").split(\",\")).mapToInt(Integer::parseInt).toArray();")
rep("PackVectors fv = new PackVectors(", "OrderVectors fv = new OrderVectors(") if "PackVectors fv = new PackVectors(" in src else None
(HERE / "OrderVectors.java").write_text(src, encoding="utf-8")
print("wrote", HERE / "OrderVectors.java")
