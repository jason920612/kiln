// Scratch: which blocks answer `isValidSpawn` differently for different entity types (kiln-entity's block table has just
// zombie and pig). usage: jv.sh ScanValidSpawn out.txt
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;

public class ScanValidSpawn {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var pos = BlockPos.ZERO;
        int n = Block.BLOCK_STATE_REGISTRY.size();
        for (int id = 0; id < n; id++) Block.BLOCK_STATE_REGISTRY.byId(id).initCache();
        Map<String, String> out = new TreeMap<>();
        for (int id = 0; id < n; id++) {
            BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
            BlockGetter one = new BlockGetter() {
                public net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(BlockPos p) { return null; }
                public BlockState getBlockState(BlockPos p) { return p.equals(pos) ? s : net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(); }
                public FluidState getFluidState(BlockPos p) { return getBlockState(p).getFluidState(); }
                public int getHeight() { return 384; }
                public int getMinY() { return -64; }
            };
            boolean zombie = s.isValidSpawn(one, pos, net.minecraft.world.entity.EntityTypes.ZOMBIE);
            StringBuilder diff = new StringBuilder();
            for (EntityType<?> t : BuiltInRegistries.ENTITY_TYPE) {
                boolean v;
                try { v = s.isValidSpawn(one, pos, t); } catch (Throwable e) { continue; }
                if (v != zombie) diff.append(' ').append(BuiltInRegistries.ENTITY_TYPE.getKey(t).getPath()).append('=').append(v);
            }
            if (diff.length() > 0) out.merge(BuiltInRegistries.BLOCK.getKey(s.getBlock()).toString() + " zombie=" + zombie, diff.toString(), (a, b) -> a);
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            out.forEach((k, v) -> w.println(k + " :" + v));
        }
        System.out.println("ScanValidSpawn done");
    }
}
