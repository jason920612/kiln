// Parity audit helper: for every registered block, which behaviour hooks its Java class chain
// overrides (randomTick, tick, neighborChanged, useWithoutItem, entityInside, ...), whether any of
// its states ticks randomly, and whether it has a block entity. One line per block:
//   <block id>\t<class chain from the block's own class up to (excluding) Block>\t<hooks>\t<flags>
// tools/parity_audit.py joins this with the references to the same classes in the Kiln sources.
//
// usage: java -cp <server jar + libraries> tools/AuditBlockBehaviour.java <out file>

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;

public class AuditBlockBehaviour {
    static final String[] HOOKS = {
        "randomTick", "tick", "neighborChanged", "useWithoutItem", "useItemOn", "entityInside", "stepOn", "fallOn",
        "onPlace", "affectNeighborsAfterRemoval", "playerWillDestroy", "playerDestroy", "triggerEvent",
        "getAnalogOutputSignal", "getSignal", "getDirectSignal", "attack", "onProjectileHit", "onExplosionHit",
        "setPlacedBy", "spawnAfterBreak", "updateEntityMovementAfterFallOn", "getTicker", "handlePrecipitation",
        "onLand", "performBonemeal", "isValidBonemealTarget", "getStateForPlacement",
    };

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (Block b : BuiltInRegistries.BLOCK) {
                String id = BuiltInRegistries.BLOCK.getKey(b).toString();
                List<String> chain = new ArrayList<>();
                Set<String> hooks = new LinkedHashSet<>();
                for (Class<?> c = b.getClass(); c != null && c != Block.class && c != BlockBehaviour.class; c = c.getSuperclass()) {
                    chain.add(c.getSimpleName());
                    for (Method m : c.getDeclaredMethods()) {
                        if (m.isSynthetic() || m.isBridge()) continue;
                        for (String h : HOOKS) {
                            if (m.getName().equals(h)) hooks.add(h);
                        }
                    }
                }
                boolean random = false;
                for (BlockState s : b.getStateDefinition().getPossibleStates()) {
                    if (s.isRandomlyTicking()) random = true;
                }
                String flags = (random ? "random " : "") + (b instanceof EntityBlock ? "blockentity " : "");
                w.println(id + "\t" + String.join(">", chain) + "\t" + String.join(",", hooks) + "\t" + flags.trim());
            }
        }
    }
}
