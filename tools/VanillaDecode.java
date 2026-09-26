// Decodes a packet body produced by Kiln with vanilla's own StreamCodec from the server jar,
// and fails on exceptions or trailing bytes. Run through tools/vanilla_decode.py.
//
// usage: java -cp <server jar + libraries> tools/VanillaDecode.java <codec-class> <body.bin>

import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;

public class VanillaDecode {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        Class<?> cls = Class.forName(args[0]);
        @SuppressWarnings("unchecked")
        StreamCodec<ByteBuf, Object> codec = (StreamCodec<ByteBuf, Object>) cls.getField("STREAM_CODEC").get(null);
        byte[] data = Files.readAllBytes(Path.of(args[1]));
        // Built-in (static) registries are enough for packets that reference entity types,
        // items, blocks, etc. Data-driven registries are not available here.
        var access = net.minecraft.core.RegistryAccess.fromRegistryOfRegistries(
                net.minecraft.core.registries.BuiltInRegistries.REGISTRY);
        ByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(data), access);
        Object packet = codec.decode(buf);
        if (buf.readableBytes() != 0) {
            System.out.println("FAIL: " + buf.readableBytes() + " trailing bytes after " + packet);
            System.exit(1);
        }
        System.out.println("OK: decoded " + data.length + " bytes as " + packet.getClass().getSimpleName());
    }
}
