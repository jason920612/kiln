// Encodes serverbound login packets with vanilla's own codecs, for Kiln's decoder tests
// (crates/kiln-net/src/testdata/vanilla_login.txt). Uses the test RSA key given as PKCS#8 hex.
//
// usage: java -cp <server jar + libraries> tools/VanillaLoginVectors.java <key.pkcs8.hex> <out.txt>
// (the classpath is the one tools/vanilla_decode.py builds)

import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.math.BigInteger;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyFactory;
import java.security.PublicKey;
import java.security.interfaces.RSAPrivateCrtKey;
import java.security.spec.PKCS8EncodedKeySpec;
import java.security.spec.RSAPublicKeySpec;
import java.util.HexFormat;
import java.util.UUID;
import javax.crypto.spec.SecretKeySpec;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.login.ServerboundCustomQueryAnswerPacket;
import net.minecraft.network.protocol.login.ServerboundHelloPacket;
import net.minecraft.network.protocol.login.ServerboundKeyPacket;
import net.minecraft.network.protocol.login.custom.CustomQueryAnswerPayload;
import net.minecraft.util.Crypt;

public class VanillaLoginVectors {
    static <T> String encode(StreamCodec<? super FriendlyByteBuf, T> codec, T packet) {
        FriendlyByteBuf buf = new FriendlyByteBuf(Unpooled.buffer());
        codec.encode(buf, packet);
        return ByteBufUtil.hexDump(buf);
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        HexFormat hex = HexFormat.of();
        byte[] pkcs8 = hex.parseHex(Files.readString(Path.of(args[0])).trim());
        KeyFactory rsa = KeyFactory.getInstance("RSA");
        RSAPrivateCrtKey priv = (RSAPrivateCrtKey) rsa.generatePrivate(new PKCS8EncodedKeySpec(pkcs8));
        byte[] publicDer = rsa.generatePublic(new RSAPublicKeySpec(priv.getModulus(), priv.getPublicExponent())).getEncoded();
        PublicKey publicKey = Crypt.byteToPublicKey(publicDer);

        byte[] secret = new byte[16];
        for (int i = 0; i < 16; i++) secret[i] = (byte) i;
        SecretKeySpec secretKey = new SecretKeySpec(secret, "AES");
        ServerboundKeyPacket key = new ServerboundKeyPacket(secretKey, publicKey, new byte[] {1, 2, 3, 4});
        String hash = new BigInteger(Crypt.digestData("", publicKey, secretKey)).toString(16);

        CustomQueryAnswerPayload payload = buf -> buf.writeBytes(new byte[] {(byte) 0xAA, (byte) 0xBB});
        StringBuilder out = new StringBuilder();
        out.append("public_key=").append(hex.formatHex(publicDer)).append('\n');
        out.append("key_packet=").append(encode(ServerboundKeyPacket.STREAM_CODEC, key)).append('\n');
        out.append("server_hash=").append(hash).append('\n');
        out.append("query_answer=").append(encode(ServerboundCustomQueryAnswerPacket.STREAM_CODEC,
                new ServerboundCustomQueryAnswerPacket(300, payload))).append('\n');
        out.append("query_answer_empty=").append(encode(ServerboundCustomQueryAnswerPacket.STREAM_CODEC,
                new ServerboundCustomQueryAnswerPacket(300, null))).append('\n');
        out.append("login_start=").append(encode(ServerboundHelloPacket.STREAM_CODEC,
                new ServerboundHelloPacket("Notch", UUID.fromString("069a79f4-44e9-4726-a5be-fca90e38aaf5")))).append('\n');
        Files.writeString(Path.of(args[1]), out);
        System.out.print(out);
    }
}
