// Encodes serverbound packets with vanilla's own codecs, for Kiln's decoder tests
// (crates/kiln-proto/tests/testdata/serverbound.txt, checked by tests/serverbound_vectors.rs).
// Each line: <state> <packet name> <case name> <body hex (no packet id)>.
//
// usage: java -cp <server jar + libraries> tools/VanillaServerboundVectors.java <out.txt>
// (the classpath is the one tools/vanilla_decode.py builds)

import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Optional;
import java.util.OptionalInt;
import java.util.UUID;
import net.minecraft.core.BlockPos;
import net.minecraft.core.PositionAndRotation;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.Vec3i;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.IntTag;
import net.minecraft.nbt.StringTag;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.ServerboundCustomClickActionPacket;
import net.minecraft.network.protocol.common.ServerboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ServerboundPongPacket;
import net.minecraft.network.protocol.common.ServerboundResourcePackPacket;
import net.minecraft.network.protocol.configuration.ServerboundAcceptCodeOfConductPacket;
import net.minecraft.network.protocol.cookie.ServerboundCookieResponsePacket;
import net.minecraft.network.protocol.game.*;
import net.minecraft.network.protocol.ping.ServerboundPingRequestPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.Difficulty;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.inventory.RecipeBookType;
import net.minecraft.world.item.crafting.display.RecipeDisplayId;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.entity.CommandBlockEntity;
import net.minecraft.world.level.block.entity.JigsawBlockEntity;
import net.minecraft.world.level.block.entity.SignTextSlot;
import net.minecraft.world.level.block.entity.StructureBlockEntity;
import net.minecraft.world.level.block.state.properties.StructureMode;
import net.minecraft.world.phys.Vec3;

public class VanillaServerboundVectors {
    static final StringBuilder OUT = new StringBuilder();
    static RegistryAccess access;

    static <T> void add(String state, String packet, String name, StreamCodec<? super RegistryFriendlyByteBuf, T> codec, T value) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        codec.encode(buf, value);
        OUT.append(state).append(' ').append(packet).append(' ').append(name).append(' ')
                .append(ByteBufUtil.hexDump(buf)).append('\n');
    }

    static <T> void play(String packet, String name, StreamCodec<? super RegistryFriendlyByteBuf, T> codec, T value) {
        add("play", packet, name, codec, value);
    }

    static Identifier id(String s) {
        return Identifier.parse(s);
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        access = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        BlockPos pos = new BlockPos(-123, -45, 6789);

        play("client_command", "respawn", ServerboundClientCommandPacket.STREAM_CODEC,
                new ServerboundClientCommandPacket(ServerboundClientCommandPacket.Action.PERFORM_RESPAWN));
        play("client_command", "stats", ServerboundClientCommandPacket.STREAM_CODEC,
                new ServerboundClientCommandPacket(ServerboundClientCommandPacket.Action.REQUEST_STATS));
        play("client_command", "game_rules", ServerboundClientCommandPacket.STREAM_CODEC,
                new ServerboundClientCommandPacket(ServerboundClientCommandPacket.Action.REQUEST_GAMERULE_VALUES));
        play("interact", "main", ServerboundInteractPacket.STREAM_CODEC,
                new ServerboundInteractPacket(300, InteractionHand.MAIN_HAND, new Vec3(0.25, 1.5, -0.125), false));
        play("interact", "off_sneak", ServerboundInteractPacket.STREAM_CODEC,
                new ServerboundInteractPacket(7, InteractionHand.OFF_HAND, Vec3.ZERO, true));
        play("interact", "far", ServerboundInteractPacket.STREAM_CODEC,
                new ServerboundInteractPacket(7, InteractionHand.MAIN_HAND, new Vec3(-3.5, 9.0, 4.0), false));
        play("attack", "attack", ServerboundAttackPacket.STREAM_CODEC, new ServerboundAttackPacket(70000));
        play("use_item", "use_item", ServerboundUseItemPacket.STREAM_CODEC,
                new ServerboundUseItemPacket(InteractionHand.OFF_HAND, 42, 90.5f, -12.25f));
        play("container_close", "close", ServerboundContainerClosePacket.STREAM_CODEC, new ServerboundContainerClosePacket(3));
        play("container_button_click", "click", ServerboundContainerButtonClickPacket.STREAM_CODEC,
                new ServerboundContainerButtonClickPacket(2, 5));
        play("container_slot_state_changed", "toggle", ServerboundContainerSlotStateChangedPacket.STREAM_CODEC,
                new ServerboundContainerSlotStateChangedPacket(4, 9, true));
        play("pick_item_from_block", "pick", ServerboundPickItemFromBlockPacket.STREAM_CODEC,
                new ServerboundPickItemFromBlockPacket(pos, true));
        play("pick_item_from_entity", "pick", ServerboundPickItemFromEntityPacket.STREAM_CODEC,
                new ServerboundPickItemFromEntityPacket(99, false));
        play("sign_update", "front", ServerboundSignUpdatePacket.STREAM_CODEC,
                new ServerboundSignUpdatePacket(pos, List.of("Hello", "", "é世界", "line 4"), SignTextSlot.FRONT));
        play("sign_update", "back", ServerboundSignUpdatePacket.STREAM_CODEC,
                new ServerboundSignUpdatePacket(pos, List.of("a", "b", "c", "d"), SignTextSlot.BACK));
        play("set_command_block", "auto", ServerboundSetCommandBlockPacket.STREAM_CODEC,
                new ServerboundSetCommandBlockPacket(pos, "say hi", CommandBlockEntity.Mode.AUTO, true, false, true));
        play("set_command_block", "redstone", ServerboundSetCommandBlockPacket.STREAM_CODEC,
                new ServerboundSetCommandBlockPacket(pos, "", CommandBlockEntity.Mode.REDSTONE, false, true, false));
        play("set_command_minecart", "minecart", ServerboundSetCommandMinecartPacket.STREAM_CODEC,
                new ServerboundSetCommandMinecartPacket(12, "time set day", true));
        play("set_structure_block", "save", ServerboundSetStructureBlockPacket.STREAM_CODEC,
                new ServerboundSetStructureBlockPacket(pos, StructureBlockEntity.UpdateType.SAVE_AREA, StructureMode.SAVE,
                        "kiln:house", new BlockPos(-3, 0, 48), new Vec3i(16, 8, 1), Mirror.FRONT_BACK,
                        Rotation.COUNTERCLOCKWISE_90, "meta", true, false, true, false, 0.75f, -987654321L));
        play("set_structure_block", "load_flags", ServerboundSetStructureBlockPacket.STREAM_CODEC,
                new ServerboundSetStructureBlockPacket(pos, StructureBlockEntity.UpdateType.LOAD_AREA, StructureMode.LOAD,
                        "", BlockPos.ZERO, Vec3i.ZERO, Mirror.NONE, Rotation.NONE, "", false, true, false, true, 1.0f, 0L));
        play("set_jigsaw_block", "jigsaw", ServerboundSetJigsawBlockPacket.STREAM_CODEC,
                new ServerboundSetJigsawBlockPacket(pos, id("minecraft:bottom"), id("kiln:top"), id("minecraft:empty"),
                        "minecraft:stone", JigsawBlockEntity.JointType.ROLLABLE, 3, -1));
        play("set_jigsaw_block", "aligned", ServerboundSetJigsawBlockPacket.STREAM_CODEC,
                new ServerboundSetJigsawBlockPacket(pos, id("a"), id("b"), id("c"), "", JigsawBlockEntity.JointType.ALIGNED, 0, 0));
        play("jigsaw_generate", "generate", ServerboundJigsawGeneratePacket.STREAM_CODEC,
                new ServerboundJigsawGeneratePacket(pos, 7, true));
        play("rename_item", "rename", ServerboundRenameItemPacket.STREAM_CODEC, new ServerboundRenameItemPacket("Excalibur"));
        play("select_trade", "trade", ServerboundSelectTradePacket.STREAM_CODEC, new ServerboundSelectTradePacket(4));
        play("set_beacon", "both", ServerboundSetBeaconPacket.STREAM_CODEC,
                new ServerboundSetBeaconPacket(Optional.of(MobEffects.SPEED), Optional.of(MobEffects.REGENERATION)));
        play("set_beacon", "none", ServerboundSetBeaconPacket.STREAM_CODEC,
                new ServerboundSetBeaconPacket(Optional.empty(), Optional.empty()));
        play("edit_book", "signed", ServerboundEditBookPacket.STREAM_CODEC,
                new ServerboundEditBookPacket(40, List.of("page one", "page two"), Optional.of("My Book")));
        play("edit_book", "unsigned", ServerboundEditBookPacket.STREAM_CODEC,
                new ServerboundEditBookPacket(0, List.of(), Optional.empty()));
        Abilities flying = new Abilities();
        flying.flying = true;
        flying.mayfly = true;
        flying.invulnerable = true;
        play("player_abilities", "flying", ServerboundPlayerAbilitiesPacket.STREAM_CODEC, new ServerboundPlayerAbilitiesPacket(flying));
        play("player_abilities", "landed", ServerboundPlayerAbilitiesPacket.STREAM_CODEC, new ServerboundPlayerAbilitiesPacket(new Abilities()));
        play("client_tick_end", "tick_end", ServerboundClientTickEndPacket.STREAM_CODEC, ServerboundClientTickEndPacket.INSTANCE);
        play("paddle_boat", "left", ServerboundPaddleBoatPacket.STREAM_CODEC, new ServerboundPaddleBoatPacket(true, false));
        play("move_vehicle", "move", ServerboundMoveVehiclePacket.STREAM_CODEC,
                new ServerboundMoveVehiclePacket(PositionAndRotation.of(new Vec3(1.5, 62.0, -7.25), 45.0f, -10.0f), true));
        play("change_difficulty", "hard", ServerboundChangeDifficultyPacket.STREAM_CODEC, new ServerboundChangeDifficultyPacket(Difficulty.HARD));
        play("lock_difficulty", "lock", ServerboundLockDifficultyPacket.STREAM_CODEC, new ServerboundLockDifficultyPacket(true));
        play("change_game_mode", "creative", ServerboundChangeGameModePacket.STREAM_CODEC, new ServerboundChangeGameModePacket(GameType.CREATIVE));
        play("teleport_to_entity", "teleport", ServerboundTeleportToEntityPacket.STREAM_CODEC,
                new ServerboundTeleportToEntityPacket(UUID.fromString("069a79f4-44e9-4726-a5be-fca90e38aaf5")));
        play("spectator_action", "spectate", ServerboundSpectatorActionPacket.STREAM_CODEC, new ServerboundSpectatorActionPacket(OptionalInt.of(42)));
        play("spectator_action", "stop", ServerboundSpectatorActionPacket.STREAM_CODEC, new ServerboundSpectatorActionPacket(OptionalInt.empty()));
        play("chat_ack", "ack", ServerboundChatAckPacket.STREAM_CODEC, new ServerboundChatAckPacket(17));
        play("bundle_item_selected", "select", ServerboundSelectBundleItemPacket.STREAM_CODEC, new ServerboundSelectBundleItemPacket(36, 2));
        play("bundle_item_selected", "deselect", ServerboundSelectBundleItemPacket.STREAM_CODEC, new ServerboundSelectBundleItemPacket(36, -1));
        play("seen_advancements", "closed", ServerboundSeenAdvancementsPacket.STREAM_CODEC, ServerboundSeenAdvancementsPacket.closedScreen());
        play("seen_advancements", "opened", ServerboundSeenAdvancementsPacket.STREAM_CODEC,
                new ServerboundSeenAdvancementsPacket(ServerboundSeenAdvancementsPacket.Action.OPENED_TAB, id("minecraft:story/root")));
        play("recipe_book_seen_recipe", "seen", ServerboundRecipeBookSeenRecipePacket.STREAM_CODEC,
                new ServerboundRecipeBookSeenRecipePacket(new RecipeDisplayId(321)));
        play("recipe_book_change_settings", "smoker", ServerboundRecipeBookChangeSettingsPacket.STREAM_CODEC,
                new ServerboundRecipeBookChangeSettingsPacket(RecipeBookType.SMOKER, true, false));
        play("place_recipe", "place", ServerboundPlaceRecipePacket.STREAM_CODEC,
                new ServerboundPlaceRecipePacket(1, new RecipeDisplayId(5), true));
        play("block_entity_tag_query", "query", ServerboundBlockEntityTagQueryPacket.STREAM_CODEC,
                new ServerboundBlockEntityTagQueryPacket(8, pos));
        play("entity_tag_query", "query", ServerboundEntityTagQueryPacket.STREAM_CODEC, new ServerboundEntityTagQueryPacket(9, 1000));
        play("set_game_rule", "rules", ServerboundSetGameRulePacket.STREAM_CODEC, new ServerboundSetGameRulePacket(List.of(
                new ServerboundSetGameRulePacket.Entry(ResourceKey.create(Registries.GAME_RULE, id("minecraft:keep_inventory")), "true"),
                new ServerboundSetGameRulePacket.Entry(ResourceKey.create(Registries.GAME_RULE, id("minecraft:random_tick_speed")), "3"))));
        play("punch", "punch", ServerboundPunchPacket.STREAM_CODEC, ServerboundPunchPacket.INSTANCE);
        play("ping_request", "ping", ServerboundPingRequestPacket.STREAM_CODEC, new ServerboundPingRequestPacket(1_700_000_000_123L));

        // Common packets, in each state they exist in.
        UUID pack = UUID.fromString("fedcba98-7654-4321-8fed-cba987654321");
        for (String state : List.of("configuration", "play")) {
            add(state, "pong", "pong", ServerboundPongPacket.STREAM_CODEC, new ServerboundPongPacket(-123456));
            add(state, "resource_pack", "accepted", ServerboundResourcePackPacket.STREAM_CODEC,
                    new ServerboundResourcePackPacket(pack, ServerboundResourcePackPacket.Action.ACCEPTED));
            add(state, "resource_pack", "discarded", ServerboundResourcePackPacket.STREAM_CODEC,
                    new ServerboundResourcePackPacket(pack, ServerboundResourcePackPacket.Action.DISCARDED));
            var payload = new CompoundTag();
            payload.put("choice", StringTag.valueOf("yes"));
            payload.put("n", IntTag.valueOf(3));
            add(state, "custom_click_action", "payload", ServerboundCustomClickActionPacket.STREAM_CODEC,
                    new ServerboundCustomClickActionPacket(id("kiln:vote"), Optional.of(payload)));
            add(state, "custom_click_action", "empty", ServerboundCustomClickActionPacket.STREAM_CODEC,
                    new ServerboundCustomClickActionPacket(id("kiln:close"), Optional.empty()));
            add(state, "custom_click_action", "string", ServerboundCustomClickActionPacket.STREAM_CODEC,
                    new ServerboundCustomClickActionPacket(id("kiln:text"), Optional.of(StringTag.valueOf("hi"))));
        }
        for (String state : List.of("login", "configuration", "play")) {
            add(state, "cookie_response", "present", ServerboundCookieResponsePacket.STREAM_CODEC,
                    new ServerboundCookieResponsePacket(id("kiln:session"), new byte[] {1, 2, 3, (byte) 0xff}));
            add(state, "cookie_response", "absent", ServerboundCookieResponsePacket.STREAM_CODEC,
                    new ServerboundCookieResponsePacket(id("kiln:session"), null));
        }
        add("configuration", "keep_alive", "keep_alive", ServerboundKeepAlivePacket.STREAM_CODEC, new ServerboundKeepAlivePacket(1L << 40));
        add("configuration", "accept_code_of_conduct", "accept", ServerboundAcceptCodeOfConductPacket.STREAM_CODEC,
                ServerboundAcceptCodeOfConductPacket.INSTANCE);

        Files.writeString(Path.of(args[0]), OUT);
        System.err.println("wrote " + OUT.toString().lines().count() + " vectors to " + args[0]);
    }
}
