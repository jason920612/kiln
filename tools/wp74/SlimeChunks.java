import net.minecraft.world.level.levelgen.WorldgenRandom;

/** Prints the slime chunks (WorldgenRandom.seedSlimeChunk(..., 987234911L).nextInt(10) == 0) with |x|,|z| <= R for a seed: SlimeChunks SEED R. */
public class SlimeChunks {
    public static void main(String[] a) {
        long seed = Long.parseLong(a[0]);
        int r = Integer.parseInt(a[1]);
        for (int x = -r; x <= r; x++) {
            for (int z = -r; z <= r; z++) {
                if (WorldgenRandom.seedSlimeChunk(x, z, seed, 987234911L).nextInt(10) == 0) {
                    System.out.println(x + " " + z);
                }
            }
        }
    }
}
