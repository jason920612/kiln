// Dumps the per-state block facts world generation reads that kiln-data's block_props lack:
// legacy solidity, sturdy faces per support type, fluid state, whether the collision shape's
// top face is full, redstone conduction, and each block's Java class chain (features dispatch
// on block classes, e.g. `instanceof DoublePlantBlock`, and canSurvive is per class).
//
// Output (little-endian): "KWBF", version 1, state count, then per state a u32 of flags
// (see crates/kiln-worldgen/src/block_facts.rs), a u8 fluid id (BuiltInRegistries.FLUID order)
// and a u8 fluid amount; then block count and per block its class chain as a string
// ("SaplingBlock<VegetationBlock<Block"), in registry order.
//
// usage: java -cp <server jar + libraries> tools/ExtractWorldgenBlocks.java <out.bin>

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SupportType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;

public class ExtractWorldgenBlocks {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var getter = EmptyBlockGetter.INSTANCE;
        var pos = BlockPos.ZERO;
        int n = Block.BLOCK_STATE_REGISTRY.size();
        for (int id = 0; id < n; id++) Block.BLOCK_STATE_REGISTRY.byId(id).initCache();
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        ByteBuffer b = ByteBuffer.allocate(16 + n * 6).order(ByteOrder.LITTLE_ENDIAN);
        b.put("KWBF".getBytes(StandardCharsets.US_ASCII)).putInt(1).putInt(n);
        SupportType[] types = {SupportType.FULL, SupportType.CENTER, SupportType.RIGID};
        for (int id = 0; id < n; id++) {
            BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
            int flags = 0;
            if (s.isSolid()) flags |= 1;
            for (Direction d : Direction.values()) {
                for (int t = 0; t < 3; t++) {
                    if (s.isFaceSturdy(getter, pos, d, types[t])) flags |= 1 << (1 + d.get3DDataValue() * 3 + t);
                }
            }
            if (Block.isFaceFull(s.getCollisionShape(getter, pos), Direction.UP)) flags |= 1 << 19;
            if (s.isRedstoneConductor(getter, pos)) flags |= 1 << 20;
            FluidState f = s.getFluidState();
            if (f.isSource()) flags |= 1 << 22;
            b.putInt(flags);
            b.put((byte) BuiltInRegistries.FLUID.getId(f.getType()));
            b.put((byte) f.getAmount());
        }
        out.write(b.array(), 0, b.position());
        ByteBuffer c = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN);
        c.putInt(BuiltInRegistries.BLOCK.size());
        out.write(c.array());
        for (Block block : BuiltInRegistries.BLOCK) {
            StringBuilder chain = new StringBuilder();
            for (Class<?> k = block.getClass(); k != null && k != Object.class; k = k.getSuperclass()) {
                if (k.getName().startsWith("net.minecraft.world.level.block.state.")) break;
                if (chain.length() > 0) chain.append('<');
                chain.append(k.getSimpleName());
            }
            byte[] v = chain.toString().getBytes(StandardCharsets.UTF_8);
            ByteBuffer l = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN);
            l.putInt(v.length);
            out.write(l.array());
            out.write(v);
        }
        Files.write(Path.of(args[0]), out.toByteArray());
        System.out.printf("%d states, %d blocks%n", n, BuiltInRegistries.BLOCK.size());
    }
}
