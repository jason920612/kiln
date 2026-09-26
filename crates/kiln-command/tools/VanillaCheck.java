// Checks kiln-command output against vanilla's own code from the server jar.
// Run through crates/kiln-command/tools/vanilla_check.py.
//
//   tree <commands.bin> <commands.json>  decode a commands packet body with vanilla's STREAM_CODEC,
//                                        rebuild the Brigadier tree the way the client does, serialize
//                                        it with ArgumentUtils.serializeNodeToJson and compare every
//                                        top-level command with the data generator's commands.json
//                                        (permissions excluded: the wire only carries "restricted").
//   show <packet-class> <body.bin>       decode a packet body with vanilla's STREAM_CODEC and print it.
//   encode                               print serverbound packet bodies encoded by vanilla (test vectors).

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.ArgumentType;
import com.mojang.brigadier.builder.ArgumentBuilder;
import com.mojang.brigadier.builder.LiteralArgumentBuilder;
import com.mojang.brigadier.builder.RequiredArgumentBuilder;
import com.mojang.brigadier.tree.CommandNode;
import com.mojang.brigadier.tree.RootCommandNode;
import io.netty.buffer.Unpooled;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.ArgumentSignatures;
import net.minecraft.commands.synchronization.ArgumentUtils;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.LastSeenMessages;
import net.minecraft.network.chat.MessageSignature;
import net.minecraft.network.protocol.game.ClientboundCommandsPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandSignedPacket;
import net.minecraft.network.protocol.game.ServerboundCommandSuggestionPacket;
import net.minecraft.resources.Identifier;

public class VanillaCheck {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        switch (args[0]) {
            case "tree" -> System.exit(tree(Path.of(args[1]), Path.of(args[2])));
            case "show" -> System.exit(show(args[1], Path.of(args[2])));
            case "encode" -> encode();
            default -> throw new IllegalArgumentException(args[0]);
        }
    }

    static int tree(Path packet, Path report) throws Exception {
        var access = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(Files.readAllBytes(packet)), access);
        ClientboundCommandsPacket decoded = ClientboundCommandsPacket.STREAM_CODEC.decode(buf);
        if (buf.readableBytes() != 0) {
            System.out.println("FAIL: " + buf.readableBytes() + " trailing bytes");
            return 1;
        }
        Map<String, String> suggestions = new TreeMap<>();
        var context = Commands.createValidationContext(VanillaRegistries.createWorldLookup());
        RootCommandNode<Object> root = decoded.getRoot(context, new ClientboundCommandsPacket.NodeBuilder<Object>() {
            public ArgumentBuilder<Object, ?> createLiteral(String name) {
                return LiteralArgumentBuilder.literal(name);
            }

            public ArgumentBuilder<Object, ?> createArgument(String name, ArgumentType<?> type, Identifier suggestion) {
                if (suggestion != null) suggestions.put(name, suggestion.toString());
                return RequiredArgumentBuilder.argument(name, type);
            }

            public ArgumentBuilder<Object, ?> configure(ArgumentBuilder<Object, ?> builder, boolean executable, boolean restricted) {
                if (executable) builder.executes(c -> 0);
                return builder;
            }
        });
        JsonObject ours = ArgumentUtils.serializeNodeToJson(new CommandDispatcher<>(root), root);
        JsonObject vanilla = JsonParser.parseString(Files.readString(report)).getAsJsonObject();
        JsonObject vanillaChildren = vanilla.getAsJsonObject("children");
        int failures = 0;
        int nodes = 0;
        List<String> compared = new ArrayList<>();
        List<String> names = new ArrayList<>(ours.getAsJsonObject("children").keySet());
        for (String name : names) {
            // Both sides go through JSON text: the report stores floats as text (1.0E-5), which
            // reads back as a double that a widened Float would not equal.
            JsonElement mine = JsonParser.parseString(ours.getAsJsonObject("children").get(name).toString());
            JsonElement theirs = vanillaChildren.get(name);
            if (theirs == null) {
                System.out.println("EXTRA (not in vanilla): " + name);
                continue;
            }
            nodes += count(mine);
            compared.add(name);
            JsonElement expected = stripPermissions(theirs.deepCopy());
            if (!expected.equals(mine)) {
                failures++;
                System.out.println("MISMATCH: " + name + "\n  ours:    " + mine + "\n  vanilla: " + expected);
            }
        }
        System.out.println("custom suggestions: " + suggestions);
        if (failures > 0) {
            System.out.println("FAIL: " + failures + " commands differ");
            return 1;
        }
        System.out.println("OK: " + compared.size() + " commands (" + nodes + " nodes) rebuilt by vanilla match commands.json: " + compared);
        return 0;
    }

    static int show(String cls, Path file) throws Exception {
        @SuppressWarnings("unchecked")
        var codec = (net.minecraft.network.codec.StreamCodec<RegistryFriendlyByteBuf, Object>) Class.forName(cls).getField("STREAM_CODEC").get(null);
        var access = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(Files.readAllBytes(file)), access);
        Object packet = codec.decode(buf);
        if (buf.readableBytes() != 0) {
            System.out.println("FAIL: " + buf.readableBytes() + " trailing bytes");
            return 1;
        }
        System.out.println("DECODED: " + packet);
        return 0;
    }

    static int count(JsonElement node) {
        int n = 1;
        JsonObject o = node.getAsJsonObject();
        if (o.has("children")) {
            for (var e : o.getAsJsonObject("children").entrySet()) n += count(e.getValue());
        }
        return n;
    }

    static JsonElement stripPermissions(JsonElement node) {
        JsonObject o = node.getAsJsonObject();
        o.remove("permissions");
        if (o.has("children")) {
            for (var e : o.getAsJsonObject("children").entrySet()) stripPermissions(e.getValue());
        }
        return o;
    }

    static void encode() {
        dump("chat_command", buf -> ServerboundChatCommandPacket.STREAM_CODEC.encode(buf, new ServerboundChatCommandPacket("gamemode creative")));
        byte[] sig = new byte[256];
        for (int i = 0; i < 256; i++) sig[i] = (byte) i;
        BitSet acked = BitSet.valueOf(new byte[] {0x05, (byte) 0xa0, 0x0c});
        dump("chat_command_signed", buf -> ServerboundChatCommandSignedPacket.STREAM_CODEC.encode(buf,
                new ServerboundChatCommandSignedPacket("say hi there", Instant.ofEpochMilli(0x19a2b3c4d5eL), -7L,
                        new ArgumentSignatures(List.of(new ArgumentSignatures.Entry("message", new MessageSignature(sig)))),
                        new LastSeenMessages.Update(3, acked, (byte) 0x2a))));
        dump("chat_command_signed_empty", buf -> ServerboundChatCommandSignedPacket.STREAM_CODEC.encode(buf,
                new ServerboundChatCommandSignedPacket("say hey", Instant.ofEpochMilli(0), 42L, ArgumentSignatures.EMPTY,
                        new LastSeenMessages.Update(0, new BitSet(), (byte) 0))));
        dump("command_suggestion", buf -> ServerboundCommandSuggestionPacket.STREAM_CODEC.encode(buf, new ServerboundCommandSuggestionPacket(128, "/teleport @")));
    }

    static void dump(String name, java.util.function.Consumer<FriendlyByteBuf> write) {
        FriendlyByteBuf buf = new FriendlyByteBuf(Unpooled.buffer());
        write.accept(buf);
        byte[] out = new byte[buf.readableBytes()];
        buf.readBytes(out);
        StringBuilder hex = new StringBuilder();
        for (byte b : out) hex.append(String.format("%02x", b));
        System.out.println(name + " " + hex);
    }
}
