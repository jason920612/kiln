// Differential test vectors for kiln-inventory, from vanilla 26.3's own menu and recipe code run
// in-process (with the vanilla datapack loaded like the dedicated server loads it).
//
// usage: java -cp <server jar + libraries> tools/InventoryVectors.java clicks <out.jsonl> <sequences> <seed>
//        java -cp <server jar + libraries> tools/InventoryVectors.java crafting <out.jsonl> <grids per recipe> <seed>
// (tools/inventory_vectors.py builds the classpath)
//
// clicks:   random inventories and menus (inventory, chests, dispenser, hopper, shulker box, crafting
//           table, furnaces), then random click sequences through a copy of
//           ServerGamePacketListenerImpl.handleContainerClick (the click packet's predicted slots are
//           derived from the real outcome, then perturbed: stale state ids, missing and bogus
//           entries). Each step records the packet, the clientbound packets vanilla sends, dropped
//           items and the resulting slots.
// crafting: for every crafting recipe, grids built from its ingredients (plus random grids), and
//           the recipe vanilla picks, its result and its remaining items.
//
// The player, level and server are allocated without running their constructors (only the
// state the menu code touches is set up), so vanilla's menu, slot and recipe classes run as is.

import com.google.common.hash.HashCode;
import com.mojang.brigadier.StringReader;
import com.mojang.serialization.DynamicOps;
import com.mojang.serialization.Lifecycle;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import it.unimi.dsi.fastutil.ints.Int2ObjectMap;
import it.unimi.dsi.fastutil.ints.Int2ObjectOpenHashMap;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.PrintStream;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Random;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.item.ItemParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.HashedPatchMap;
import net.minecraft.network.HashedStack;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundContainerSetContentPacket;
import net.minecraft.network.protocol.game.ClientboundContainerSetDataPacket;
import net.minecraft.network.protocol.game.ClientboundContainerSetSlotPacket;
import net.minecraft.network.protocol.game.ClientboundSetCursorItemPacket;
import net.minecraft.network.protocol.game.ClientboundSetPlayerInventoryPacket;
import net.minecraft.network.protocol.game.ServerboundContainerClickPacket;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.ReloadableServerResources;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.ServerGamePacketListenerImpl;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.stats.Stat;
import net.minecraft.util.HashOps;
import net.minecraft.util.Prediction;
import net.minecraft.util.Util;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.EntityEquipment;

import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.BlastFurnaceMenu;
import net.minecraft.world.inventory.ChestMenu;
import net.minecraft.world.inventory.ContainerInput;
import net.minecraft.world.inventory.ContainerLevelAccess;
import net.minecraft.world.inventory.ContainerListener;
import net.minecraft.world.inventory.ContainerSynchronizer;
import net.minecraft.world.inventory.CraftingMenu;
import net.minecraft.world.inventory.DispenserMenu;
import net.minecraft.world.inventory.FurnaceMenu;
import net.minecraft.world.inventory.HopperMenu;
import net.minecraft.world.inventory.InventoryMenu;
import net.minecraft.world.inventory.MenuType;
import net.minecraft.world.inventory.RemoteSlot;
import net.minecraft.world.inventory.ShulkerBoxMenu;
import net.minecraft.world.inventory.SimpleContainerData;
import net.minecraft.world.inventory.SmithingMenu;
import net.minecraft.world.inventory.SmokerMenu;
import net.minecraft.world.inventory.StonecutterMenu;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.crafting.CraftingInput;
import net.minecraft.world.item.crafting.CraftingRecipe;
import net.minecraft.world.item.crafting.Ingredient;
import net.minecraft.world.item.crafting.RecipeHolder;
import net.minecraft.world.item.crafting.RecipeManager;
import net.minecraft.world.item.crafting.RecipeType;
import net.minecraft.world.item.crafting.ShapedRecipe;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.saveddata.maps.MapId;
import net.minecraft.world.level.saveddata.maps.MapItemSavedData;
import sun.misc.Unsafe;

public class InventoryVectors {
    static final PrintStream OUT = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);
    static RegistryAccess access;
    static ReloadableServerResources resources;
    static RecipeManager recipes;
    static DynamicOps<HashCode> hashOps;
    static Unsafe U;
    static FakeLevel LEVEL;
    static MinecraftServer SERVER;
    static GameRules RULES;
    static final Map<Integer, MapItemSavedData> MAPS = new HashMap<>();
    static final HashedPatchMap.HashGenerator HASHER = c -> c.encodeValue(hashOps).getOrThrow().asInt();

    record Loaded(ReloadableServerResources resources, RegistryAccess.Frozen access) {}

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        load();
        setup();
        switch (args[0]) {
            case "clicks" -> clicks(Path.of(args[1]), Integer.parseInt(args[2]), Long.parseLong(args[3]));
            case "crafting" -> crafting(Path.of(args[1]), Integer.parseInt(args[2]), Long.parseLong(args[3]));
            case "sync" -> sync(Path.of(args[1]));
            default -> throw new IllegalArgumentException("unknown mode " + args[0]);
        }
    }

    static void load() throws Exception {
        PackRepository repo = ServerPacksSource.createVanillaTrustedRepository();
        var packs = new WorldLoader.PackConfig(repo, WorldDataConfiguration.DEFAULT, false, true);
        var init = new WorldLoader.InitConfig(packs, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
        Loaded loaded = WorldLoader.<Void, Loaded>load(init, ctx -> {
            Registry<LevelStem> none = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
            var dims = ctx.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                    .getOrThrow(WorldPresets.NORMAL).value().createWorldDimensions().bake(none);
            return new WorldLoader.DataLoadOutput<>(null, dims.dimensionsRegistryAccess());
        }, (manager, res, layers, cookie) -> {
            manager.close();
            return new Loaded(res, layers.compositeAccess());
        }, Util.backgroundExecutor(), Runnable::run).get();
        loaded.resources().updateComponentsAndStaticRegistryTags();
        access = loaded.access();
        resources = loaded.resources();
        recipes = resources.getRecipeManager();
        recipes.finalizeRecipeLoading(FeatureFlags.DEFAULT_FLAGS);
        hashOps = access.createSerializationContext(HashOps.CRC32C_INSTANCE);
    }

    static void set(Object target, Class<?> owner, String field, Object value) throws Exception {
        Field f = owner.getDeclaredField(field);
        f.setAccessible(true);
        f.set(target, value);
    }

    static void setup() throws Exception {
        Field uf = Unsafe.class.getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        U = (Unsafe) uf.get(null);
        SERVER = (MinecraftServer) U.allocateInstance(DedicatedServer.class);
        Class<?> rr = Class.forName("net.minecraft.server.MinecraftServer$ReloadableResources");
        var ctor = rr.getDeclaredConstructors()[0];
        ctor.setAccessible(true);
        set(SERVER, MinecraftServer.class, "resources", ctor.newInstance(null, resources));
        LEVEL = (FakeLevel) U.allocateInstance(FakeLevel.class);
        set(LEVEL, net.minecraft.world.level.Level.class, "registryAccess", access);
        set(LEVEL, net.minecraft.world.level.Level.class, "random", net.minecraft.util.RandomSource.create(0));
        RULES = new GameRules(FeatureFlags.DEFAULT_FLAGS);
    }

    // ---- fakes ----------------------------------------------------------------------------------

    static class FakeLevel extends ServerLevel {
        FakeLevel() {
            super(null, null, null, null, null, null, false, 0L, null, false);
        }

        @Override
        public MinecraftServer getServer() {
            return SERVER;
        }

        @Override
        public RecipeManager recipeAccess() {
            return recipes;
        }

        @Override
        public FeatureFlagSet enabledFeatures() {
            return FeatureFlags.DEFAULT_FLAGS;
        }

        @Override
        public GameRules getGameRules() {
            return RULES;
        }

        @Override
        public MapItemSavedData getMapData(MapId id) {
            return MAPS.get(id.id());
        }

        @Override
        public long getGameTime() {
            return 0;
        }

        @Override
        public void playSound(net.minecraft.world.entity.Entity e, BlockPos pos, net.minecraft.sounds.SoundEvent sound, net.minecraft.sounds.SoundSource source, float volume, float pitch) {}

        @Override
        public void levelEvent(net.minecraft.world.entity.Entity e, int type, BlockPos pos, int data) {}
    }

    static class FakePlayer extends ServerPlayer {
        GameType mode;
        List<String> sink;

        FakePlayer() {
            super(null, null, null, null);
        }

        @Override
        public ServerLevel level() {
            return LEVEL;
        }

        @Override
        public GameType gameMode() {
            return mode;
        }

        @Override
        public ItemEntity drop(ItemStack stack, boolean retainOwnership, Prediction prediction) {
            if (!stack.isEmpty()) {
                sink.add("{\"drop\": \"" + hex(stack) + "\", \"retain\": " + retainOwnership + "}");
            }
            return null;
        }

        @Override
        public void onEquipItem(EquipmentSlot slot, ItemStack old, ItemStack now) {}

        @Override
        public void playSound(net.minecraft.sounds.SoundEvent sound, float volume, float pitch) {}

        @Override
        public void awardStat(Stat<?> stat, int amount) {}

        @Override
        public int awardRecipes(Collection<RecipeHolder<?>> r) {
            return 0;
        }

        @Override
        public void triggerRecipeCrafted(RecipeHolder<?> r, List<ItemStack> items) {}
    }

    static class FakeConnection extends ServerGamePacketListenerImpl {
        List<String> sink;

        FakeConnection() {
            super(null, null, null, null);
        }

        @Override
        public void send(Packet<?> p) {
            sink.add(packetJson(p));
        }
    }

    static class Sync implements ContainerSynchronizer {
        final FakeConnection conn;

        Sync(FakeConnection conn) {
            this.conn = conn;
        }

        public void sendInitialData(AbstractContainerMenu m, List<ItemStack> items, ItemStack carried, int[] data) {
            conn.send(new ClientboundContainerSetContentPacket(m.containerId, m.incrementStateId(), items, carried));
            for (int i = 0; i < data.length; i++) {
                conn.send(new ClientboundContainerSetDataPacket(m.containerId, i, data[i]));
            }
        }

        public void sendSlotChange(AbstractContainerMenu m, int slot, ItemStack stack) {
            conn.send(new ClientboundContainerSetSlotPacket(m.containerId, m.incrementStateId(), slot, stack));
        }

        public void sendCarriedChange(AbstractContainerMenu m, ItemStack stack) {
            conn.send(new ClientboundSetCursorItemPacket(stack));
        }

        public void sendDataChange(AbstractContainerMenu m, int id, int value) {
            conn.send(new ClientboundContainerSetDataPacket(m.containerId, id, value));
        }

        public RemoteSlot createSlot() {
            return new RemoteSlot.Synchronized(HASHER);
        }
    }

    static final ContainerListener NO_LISTENER = new ContainerListener() {
        public void slotChanged(AbstractContainerMenu m, int slot, ItemStack stack) {}

        public void dataChanged(AbstractContainerMenu m, int id, int value) {}
    };

    // ---- encoding -------------------------------------------------------------------------------

    static String hex(ItemStack s) {
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, s);
        return ByteBufUtil.hexDump(buf);
    }

    static String packetJson(Packet<?> p) {
        if (p instanceof ClientboundContainerSetSlotPacket s) {
            return "{\"set_slot\": [" + s.getContainerId() + ", " + s.getStateId() + ", " + s.getSlot() + ", \"" + hex(s.getItem()) + "\"]}";
        }
        if (p instanceof ClientboundContainerSetContentPacket s) {
            List<String> items = new ArrayList<>();
            for (ItemStack i : s.items()) items.add("\"" + hex(i) + "\"");
            return "{\"set_content\": [" + s.containerId() + ", " + s.stateId() + ", [" + String.join(", ", items) + "], \"" + hex(s.carriedItem()) + "\"]}";
        }
        if (p instanceof ClientboundSetCursorItemPacket s) {
            return "{\"set_cursor\": \"" + hex(s.contents()) + "\"}";
        }
        if (p instanceof ClientboundSetPlayerInventoryPacket s) {
            return "{\"set_player_inventory\": [" + s.slot() + ", \"" + hex(s.contents()) + "\"]}";
        }
        if (p instanceof ClientboundContainerSetDataPacket s) {
            return "{\"set_data\": [" + s.getContainerId() + ", " + s.getId() + ", " + s.getValue() + "]}";
        }
        return "{\"other\": \"" + p.getClass().getSimpleName() + "\"}";
    }

    static String hexList(List<ItemStack> stacks) {
        List<String> out = new ArrayList<>();
        for (ItemStack s : stacks) out.add("\"" + hex(s) + "\"");
        return "[" + String.join(", ", out) + "]";
    }

    // ---- stacks ---------------------------------------------------------------------------------

    static ItemParser parser;

    static ItemStack parse(String text, int count) {
        try {
            if (parser == null) parser = new ItemParser(access);
            var input = parser.parse(new StringReader(text));
            return new ItemStack(input.item(), count, input.components());
        } catch (Exception e) {
            throw new RuntimeException(text + ": " + e.getMessage(), e);
        }
    }

    static final String[] CLICK_POOL = {
        "stone", "dirt", "cobblestone", "oak_planks", "stick", "ender_pearl", "snowball", "egg", "bucket", "water_bucket",
        "diamond_sword", "iron_pickaxe[damage=5]", "shield", "bow", "totem_of_undying", "iron_helmet", "diamond_chestplate",
        "leather_leggings", "golden_boots", "carved_pumpkin", "turtle_helmet", "elytra", "wolf_armor", "saddle",
        "iron_helmet[enchantments={binding_curse:1}]", "chainmail_boots[enchantments={binding_curse:1,unbreaking:2}]",
        "stone[custom_name=\"A\"]", "stone[max_stack_size=5]", "stone[max_stack_size=99]", "dirt[max_stack_size=1]",
        "diamond_sword[damage=100]", "shulker_box", "red_shulker_box", "coal", "charcoal", "raw_iron", "beef", "lava_bucket",
        "oak_log", "iron_ingot", "sugar", "wheat", "milk_bucket", "honey_bottle", "paper", "gunpowder", "red_dye", "blue_dye",
        "bamboo", "blaze_rod", "stone[repair_cost=3]", "ender_pearl[!max_stack_size]", "player_head", "potion",
        "bundle", "bundle", "bundle[bundle_contents=[{id:\"stone\",count:10}]]", "red_bundle[bundle_contents=[{id:\"ender_pearl\",count:4},{id:\"diamond_sword\"}]]",
        "blue_bundle[bundle_contents=[{id:\"bundle\",components:{\"minecraft:bundle_contents\":[{id:\"dirt\",count:3}]}}]]", "bundle[bundle_contents=[{id:\"oak_planks\",count:64}]]",
    };

    static final String[] CRAFT_POOL = {
        "oak_log", "oak_planks", "stick", "cobblestone", "iron_ingot", "gold_ingot", "diamond", "redstone", "string", "wheat",
        "sugar", "egg", "milk_bucket", "water_bucket", "honey_bottle", "glass_bottle", "paper", "gunpowder", "leather",
        "firework_star[firework_explosion={shape:\"star\",colors:[I;255]}]", "leather_chestplate", "leather_helmet[dyed_color=1234]",
        "red_dye", "blue_dye", "white_dye", "white_banner", "white_banner[banner_patterns=[{pattern:\"minecraft:stripe_top\",color:\"red\"}]]",
        "shield", "shulker_box", "writable_book", "written_book[written_book_content={title:\"t\",author:\"a\",generation:1,pages:[\"x\"]}]",
        "arrow", "lingering_potion[potion_contents={potion:\"minecraft:swiftness\"}]", "brick", "angler_pottery_sherd",
        "wooden_pickaxe[damage=10]", "wooden_pickaxe[damage=40]", "stone_sword", "book", "ink_sac", "feather", "slime_ball",
        "iron_nugget", "coal", "blaze_powder", "ender_pearl", "glowstone_dust", "fire_charge", "gold_nugget", "bundle",
        "bundle[bundle_contents=[{id:\"stick\",count:7}]]",
    };

    static final String[] STONE_POOL = {
        "stone", "cobblestone", "granite", "andesite", "diorite", "deepslate", "cobbled_deepslate", "copper_block", "cut_copper",
        "exposed_copper", "sandstone", "red_sandstone", "quartz_block", "blackstone", "polished_blackstone", "stone_bricks", "tuff",
        "bricks", "prismarine", "purpur_block", "end_stone", "mud_bricks", "nether_bricks", "oak_planks", "dirt", "iron_ingot",
        "stone[custom_name=\"A\"]", "stone[max_stack_size=5]", "smooth_stone", "resin_bricks", "bundle", "stone_slab",
    };

    static final String[] SMITH_POOL = {
        "netherite_upgrade_smithing_template", "coast_armor_trim_smithing_template", "wild_armor_trim_smithing_template",
        "sentry_armor_trim_smithing_template", "diamond_sword", "diamond_chestplate", "diamond_helmet", "diamond_pickaxe[damage=100,enchantments={efficiency:3}]",
        "diamond_helmet[trim={material:\"minecraft:iron\",pattern:\"minecraft:coast\"}]", "iron_chestplate", "leather_boots[dyed_color=255]",
        "turtle_helmet", "chainmail_leggings", "golden_boots", "netherite_chestplate", "diamond_hoe[custom_name=\"H\"]", "netherite_ingot",
        "iron_ingot", "gold_ingot", "emerald", "redstone", "amethyst_shard", "quartz", "copper_ingot", "lapis_lazuli", "diamond",
        "resin_brick", "stone", "diamond_leggings[max_stack_size=4]", "netherite_upgrade_smithing_template[max_stack_size=1]",
    };

    static ItemStack randomStack(Random rng, String[] pool) {
        String text = pool[rng.nextInt(pool.length)];
        ItemStack s = parse(text, 1);
        int max = s.getMaxStackSize();
        int count;
        int r = rng.nextInt(20);
        if (r == 0) count = 1 + rng.nextInt(99);
        else if (r < 4) count = max;
        else count = 1 + rng.nextInt(max);
        s.setCount(count);
        return s;
    }

    // ---- clicks ---------------------------------------------------------------------------------

    static final String[] KINDS = {
        "inventory", "inventory", "inventory", "crafting", "crafting", "generic_9x3", "generic_9x6", "generic_9x1",
        "generic_3x3", "hopper", "shulker_box", "furnace", "blast_furnace", "smoker", "stonecutter", "stonecutter", "smithing", "smithing",
    };

    static int blockSize(String kind) {
        return switch (kind) {
            case "generic_9x1" -> 9;
            case "generic_9x3", "shulker_box" -> 27;
            case "generic_9x6" -> 54;
            case "generic_3x3" -> 9;
            case "hopper" -> 5;
            case "furnace", "blast_furnace", "smoker" -> 3;
            default -> 0;
        };
    }

    static final class Session {
        FakePlayer player;
        Inventory inv;
        SimpleContainer block;
        SimpleContainerData data;
        AbstractContainerMenu menu;
        List<String> sink = new ArrayList<>();
        String kind;
    }

    static Session session(String kind, int containerId, boolean creative) throws Exception {
        Session s = new Session();
        s.kind = kind;
        FakePlayer p = (FakePlayer) U.allocateInstance(FakePlayer.class);
        p.mode = creative ? GameType.CREATIVE : GameType.SURVIVAL;
        p.sink = s.sink;
        Abilities abilities = new Abilities();
        abilities.instabuild = creative;
        set(p, net.minecraft.world.entity.player.Player.class, "abilities", abilities);
        set(p, net.minecraft.world.entity.Entity.class, "type", net.minecraft.world.entity.EntityTypes.PLAYER);
        s.inv = new Inventory(p, new EntityEquipment());
        set(p, net.minecraft.world.entity.player.Player.class, "inventory", s.inv);
        FakeConnection conn = (FakeConnection) U.allocateInstance(FakeConnection.class);
        conn.sink = s.sink;
        p.connection = conn;
        s.player = p;
        int n = blockSize(kind);
        s.block = new SimpleContainer(Math.max(n, 1));
        s.data = new SimpleContainerData(4);
        return s;
    }

    static AbstractContainerMenu createMenu(Session s, int id) {
        return switch (s.kind) {
            case "inventory" -> new InventoryMenu(s.inv, true, s.player);
            case "generic_9x1" -> new ChestMenu(MenuType.GENERIC_9x1, id, s.inv, s.block, 1);
            case "generic_9x3" -> new ChestMenu(MenuType.GENERIC_9x3, id, s.inv, s.block, 3);
            case "generic_9x6" -> new ChestMenu(MenuType.GENERIC_9x6, id, s.inv, s.block, 6);
            case "generic_3x3" -> new DispenserMenu(id, s.inv, s.block);
            case "hopper" -> new HopperMenu(id, s.inv, s.block);
            case "shulker_box" -> new ShulkerBoxMenu(id, s.inv, s.block);
            case "crafting" -> new CraftingMenu(id, s.inv, ContainerLevelAccess.create(LEVEL, BlockPos.ZERO));
            case "furnace" -> new FurnaceMenu(id, s.inv, s.block, s.data);
            case "blast_furnace" -> new BlastFurnaceMenu(id, s.inv, s.block, s.data);
            case "smoker" -> new SmokerMenu(id, s.inv, s.block, s.data);
            case "stonecutter" -> new StonecutterMenu(id, s.inv, ContainerLevelAccess.create(LEVEL, BlockPos.ZERO));
            case "smithing" -> new SmithingMenu(id, s.inv, ContainerLevelAccess.create(LEVEL, BlockPos.ZERO));
            default -> throw new IllegalArgumentException(s.kind);
        };
    }

    static String state(Session s) {
        List<ItemStack> inv = new ArrayList<>();
        for (int i = 0; i < 43; i++) inv.add(s.inv.getItem(i));
        return "{\"slots\": " + hexList(s.menu.getItems()) + ", \"carried\": \"" + hex(s.menu.getCarried()) + "\", \"inv\": " + hexList(inv)
                + ", \"block\": " + hexList(s.block.getItems()) + ", \"state_id\": " + s.menu.getStateId() + "}";
    }

    static String drain(Session s) {
        String out = "[" + String.join(", ", s.sink) + "]";
        s.sink.clear();
        return out;
    }

    static void clicks(Path out, int sequences, long seed) throws Exception {
        Random rng = new Random(seed);
        int crashes = 0;
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8))) {
            for (int n = 0; n < sequences; n++) {
                String kind = KINDS[rng.nextInt(KINDS.length)];
                boolean creative = rng.nextInt(5) == 0;
                int id = kind.equals("inventory") ? 0 : 1 + rng.nextInt(100);
                Session s = session(kind, id, creative);
                String[] pool = kind.equals("crafting") || (kind.equals("inventory") && rng.nextBoolean()) ? CRAFT_POOL
                        : kind.equals("stonecutter") ? STONE_POOL : kind.equals("smithing") ? SMITH_POOL : CLICK_POOL;
                double fill = rng.nextDouble();
                for (int i = 0; i < 43; i++) {
                    if (rng.nextDouble() < fill * 0.8) s.inv.setItem(i, randomStack(rng, pool));
                }
                s.inv.setSelectedSlot(rng.nextInt(9));
                for (int i = 0; i < s.block.getContainerSize(); i++) {
                    if (rng.nextDouble() < fill * 0.6) s.block.getItems().set(i, randomStack(rng, pool));
                }
                for (int i = 0; i < 4; i++) s.data.set(i, rng.nextInt(300));
                List<ItemStack> inv0 = new ArrayList<>();
                for (int i = 0; i < 43; i++) inv0.add(s.inv.getItem(i).copy());
                List<ItemStack> block0 = new ArrayList<>();
                for (ItemStack b : s.block.getItems()) block0.add(b.copy());
                s.menu = createMenu(s, id);
                s.player.containerMenu = s.menu;
                List<ItemStack> grid = new ArrayList<>();
                int gridSize = kind.equals("crafting") ? 3 : kind.equals("inventory") ? 2 : 0;
                if (gridSize > 0 && rng.nextInt(5) < 3) {
                    List<ItemStack> g = null;
                    for (int tries = 0; g == null && tries < 20; tries++) {
                        RecipeHolder<?> h = craftingRecipes().get(rng.nextInt(craftingRecipes().size()));
                        g = gridFor((CraftingRecipe) h.value(), h.id().identifier().getPath(), gridSize, rng);
                    }
                    if (g != null) {
                        for (ItemStack st : g) {
                            if (!st.isEmpty() && rng.nextInt(3) > 0) st.setCount(1 + rng.nextInt(st.getMaxStackSize()));
                        }
                        grid = g;
                    }
                }
                boolean station = kind.equals("stonecutter") || kind.equals("smithing");
                if (station && rng.nextInt(4) < 3) grid = stationInput(kind, rng);
                for (int i = 0; i < grid.size(); i++) {
                    if (!grid.get(i).isEmpty()) s.menu.getSlot((station ? 0 : 1) + i).set(grid.get(i).copy());
                }
                // Deterministic drag order (a HashSet of slots in vanilla iterates in identity-hash order).
                set(s.menu, AbstractContainerMenu.class, "quickcraftSlots", new LinkedHashSet<>());
                s.menu.addSlotListener(NO_LISTENER);
                s.menu.setSynchronizer(new Sync((FakeConnection) s.player.connection));
                StringBuilder b = new StringBuilder();
                b.append("{\"kind\": \"").append(kind).append("\", \"container_id\": ").append(id).append(", \"creative\": ").append(creative)
                        .append(", \"selected\": ").append(s.inv.getSelectedSlot()).append(", \"inv\": ").append(hexList(inv0))
                        .append(", \"block\": ").append(hexList(block0)).append(", \"grid\": ").append(hexList(grid)).append(", \"data\": [").append(s.data.get(0)).append(", ").append(s.data.get(1))
                        .append(", ").append(s.data.get(2)).append(", ").append(s.data.get(3)).append("], \"open\": ").append(drain(s)).append(", \"steps\": [");
                int steps = 5 + rng.nextInt(40);
                int drag = -1;
                boolean first = true;
                for (int k = 0; k < steps; k++) {
                    String step;
                    if (kind.equals("inventory") && drag < 0 && rng.nextInt(25) == 0) {
                        step = creativeStep(s, rng, pool);
                    } else if (drag < 0 && rng.nextInt(kind.equals("stonecutter") ? 4 : 60) == 0) {
                        step = buttonStep(s, rng);
                    } else if (drag < 0 && k == steps - 1 && rng.nextInt(3) == 0) {
                        s.menu.removed(s.player);
                        step = "{\"close\": true, \"out\": " + drain(s) + ", \"state\": " + state(s) + "}";
                    } else {
                        int[] c = nextClick(s, rng, drag);
                        drag = c[3];
                        step = clickStep(s, rng, c[0], c[1], ContainerInput.values()[c[2]]);
                        if (step.contains("\"crash\": true")) {
                            crashes++;
                            b.append(first ? "" : ", ").append(step);
                            break;
                        }
                    }
                    b.append(first ? "" : ", ").append(step);
                    first = false;
                    if (step.startsWith("{\"close\"")) break;
                }
                w.println(b.append("]}"));
            }
        }
        OUT.println("wrote " + sequences + " sequences (" + crashes + " ending in a crash) to " + out);
    }

    /** Returns {slot, button, input ordinal, drag type after this click (-1 when none)}. */
    static int[] nextClick(Session s, Random rng, int drag) {
        int size = s.menu.slots.size();
        if (drag >= 0) {
            if (rng.nextInt(4) == 0) return new int[] {-999, AbstractContainerMenu.getQuickcraftMask(2, drag), 5, -1};
            if (rng.nextInt(30) == 0) return new int[] {rng.nextInt(size), 0, 0, -1};
            return new int[] {rng.nextInt(size), AbstractContainerMenu.getQuickcraftMask(1, drag), 5, drag};
        }
        int slot = rng.nextInt(20) == 0 ? -999 : rng.nextInt(size);
        if ((s.kind.equals("crafting") || s.kind.equals("inventory")) && rng.nextInt(5) == 0) slot = rng.nextInt(3) == 0 ? 1 + rng.nextInt(4) : 0;
        if (s.kind.equals("stonecutter") && rng.nextInt(3) == 0) slot = rng.nextInt(2);
        if (s.kind.equals("smithing") && rng.nextInt(3) == 0) slot = rng.nextInt(4);
        if (rng.nextInt(200) == 0) slot = size + rng.nextInt(3);
        if (rng.nextInt(300) == 0) slot = -1 - rng.nextInt(3);
        int r = rng.nextInt(100);
        if (r < 40) return new int[] {slot, rng.nextInt(10) == 0 ? 2 : rng.nextInt(2), 0, -1};
        if (r < 55) return new int[] {slot, rng.nextInt(2), 1, -1};
        if (r < 67) {
            int button = rng.nextInt(12) == 0 ? 40 : rng.nextInt(9);
            if (rng.nextInt(40) == 0) button = 9 + rng.nextInt(40);
            return new int[] {slot == -999 && rng.nextInt(4) > 0 ? rng.nextInt(size) : slot, button, 2, -1};
        }
        if (r < 71) return new int[] {slot, 2, 3, -1};
        if (r < 77) return new int[] {slot, rng.nextInt(2), 4, -1};
        if (r < 92) {
            int type = rng.nextInt(10) == 0 ? 2 : rng.nextInt(2);
            if (rng.nextInt(30) == 0) type = 3;
            return new int[] {-999, AbstractContainerMenu.getQuickcraftMask(0, type), 5, type};
        }
        if (r < 97) return new int[] {slot, rng.nextInt(2), 6, -1};
        return new int[] {slot, rng.nextInt(3), rng.nextInt(7), -1};
    }

    static String clickStep(Session s, Random rng, int slot, int button, ContainerInput input) {
        AbstractContainerMenu m = s.menu;
        int containerId = rng.nextInt(200) == 0 ? m.containerId + 1 : m.containerId;
        int stateId = rng.nextInt(8) == 0 ? rng.nextInt(Math.max(1, m.getStateId() + 1)) : m.getStateId();
        List<ItemStack> before = new ArrayList<>();
        for (ItemStack i : m.getItems()) before.add(i.copy());
        ItemStack carriedBefore = m.getCarried().copy();
        boolean crash = false;
        Int2ObjectMap<HashedStack> changed = new Int2ObjectOpenHashMap<>();
        HashedStack carried = HashedStack.EMPTY;
        if (m.containerId == containerId && m.isValidSlotIndex(slot)) {
            boolean full = stateId != m.getStateId();
            m.suppressRemoteUpdates();
            try {
                m.clicked(slot, button, input, s.player);
            } catch (RuntimeException e) {
                crash = true;
                if (System.getenv("KILN_TRACE") != null) {
                    Throwable t = e;
                    while (t.getCause() != null) t = t.getCause();
                    OUT.println("crash: " + t);
                    for (StackTraceElement el : t.getStackTrace()) OUT.println("  at " + el);
                }
            }
            if (!crash) {
                List<ItemStack> after = m.getItems();
                for (int i = 0; i < after.size(); i++) {
                    if (!ItemStack.matches(before.get(i), after.get(i))) changed.put(i, HashedStack.create(after.get(i), HASHER));
                }
                carried = HashedStack.create(m.getCarried(), HASHER);
                perturb(rng, m, changed);
                if (rng.nextInt(10) == 0) carried = HashedStack.create(randomStack(rng, CLICK_POOL), HASHER);
                for (var e : changed.int2ObjectEntrySet()) m.setRemoteSlotUnsafe(e.getIntKey(), e.getValue());
                m.setRemoteCarried(carried);
                m.resumeRemoteUpdates();
                if (full) m.broadcastFullState();
                else m.broadcastChanges();
            }
        } else {
            carried = HashedStack.create(carriedBefore, HASHER);
        }
        var packet = new ServerboundContainerClickPacket(containerId, stateId, (short) slot, (byte) button, input, changed, carried);
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        ServerboundContainerClickPacket.STREAM_CODEC.encode(buf, packet);
        String click = ByteBufUtil.hexDump(buf);
        if (crash) {
            s.sink.clear();
            return "{\"click\": \"" + click + "\", \"crash\": true}";
        }
        return "{\"click\": \"" + click + "\", \"out\": " + drain(s) + ", \"state\": " + state(s) + "}";
    }

    static void perturb(Random rng, AbstractContainerMenu m, Int2ObjectMap<HashedStack> changed) {
        if (!changed.isEmpty() && rng.nextInt(8) == 0) {
            int k = changed.keySet().iterator().nextInt();
            changed.remove(k);
        }
        if (rng.nextInt(8) == 0) {
            changed.put(rng.nextInt(m.slots.size()), HashedStack.create(randomStack(rng, CLICK_POOL), HASHER));
        }
        if (rng.nextInt(20) == 0) changed.put(m.slots.size() + rng.nextInt(5), HashedStack.EMPTY);
        if (rng.nextInt(30) == 0) changed.put(-1 - rng.nextInt(3), HashedStack.EMPTY);
    }

    /** A copy of handleContainerButtonClick (the menu is always still valid). */
    static String buttonStep(Session s, Random rng) {
        AbstractContainerMenu m = s.menu;
        int containerId = rng.nextInt(30) == 0 ? m.containerId + 1 : m.containerId;
        int visible = m instanceof StonecutterMenu sc ? sc.getNumberOfVisibleRecipes() : 2;
        int button = rng.nextInt(10) == 0 ? rng.nextInt(8) - 3 : rng.nextInt(visible + 2);
        if (m.containerId == containerId && m.clickMenuButton(s.player, button)) m.broadcastChanges();
        return "{\"button\": [" + containerId + ", " + button + "], \"out\": " + drain(s) + ", \"state\": " + state(s) + "}";
    }

    /** A copy of handleSetCreativeModeSlot on the inventory menu (drops never throttled). */
    static String creativeStep(Session s, Random rng, String[] pool) {
        int slot = rng.nextInt(10) == 0 ? -1 : rng.nextInt(10) == 0 ? rng.nextInt(60) - 5 : 1 + rng.nextInt(45);
        ItemStack stack = rng.nextInt(6) == 0 ? ItemStack.EMPTY : randomStack(rng, pool);
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC.encode(buf, stack);
        String encoded = ByteBufUtil.hexDump(buf);
        if (s.player.hasInfiniteMaterials()) {
            boolean drop = slot < 0;
            boolean valid = stack.isEmpty() || stack.getCount() <= stack.getMaxStackSize();
            if (slot >= 1 && slot <= 45 && valid) {
                s.menu.getSlot(slot).setByPlayer(stack);
                s.menu.setRemoteSlot(slot, stack);
                s.menu.broadcastChanges();
            } else if (drop && valid) {
                s.player.drop(stack, true, Prediction.PREDICTED);
            }
        }
        return "{\"creative\": [" + slot + ", \"" + encoded + "\"], \"out\": " + drain(s) + ", \"state\": " + state(s) + "}";
    }

    // ---- recipe sync ----------------------------------------------------------------------------

    static void sync(Path out) throws Exception {
        var packet = new net.minecraft.network.protocol.game.ClientboundUpdateRecipesPacket(recipes.getSynchronizedItemProperties(),
                recipes.getSynchronizedStonecutterRecipes());
        var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
        net.minecraft.network.protocol.game.ClientboundUpdateRecipesPacket.STREAM_CODEC.encode(buf, packet);
        Files.writeString(out, "{\"update_recipes\": \"" + ByteBufUtil.hexDump(buf) + "\"}\n", StandardCharsets.UTF_8);
        OUT.println("wrote " + out);
    }

    // ---- crafting -------------------------------------------------------------------------------

    static void crafting(Path out, int perRecipe, long seed) throws Exception {
        Random rng = new Random(seed);
        for (int i = 0; i < 6; i++) MAPS.put(i, MapItemSavedData.createFresh(0, 0, (byte) Math.min(i, 4), false, false, net.minecraft.world.level.Level.OVERWORLD));
        int grids = 0;
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8))) {
            List<String> order = new ArrayList<>();
            for (RecipeHolder<?> h : recipes.getRecipes()) order.add("\"" + h.id().identifier() + "\"");
            w.println("{\"order\": [" + String.join(", ", order) + "]}");
            for (RecipeHolder<?> h : recipes.getRecipes()) {
                if (!(h.value() instanceof CraftingRecipe recipe)) continue;
                boolean special = recipe.placementInfo().isImpossibleToPlace() || recipe.getClass().getSimpleName().matches("DyeRecipe|ImbueRecipe");
                for (int k = 0; k < (special ? perRecipe * 6 : perRecipe); k++) {
                    for (int size : new int[] {3, 2}) {
                        List<ItemStack> grid = gridFor(recipe, h.id().identifier().getPath(), size, rng);
                        if (grid == null) continue;
                        w.println(craftRecord(h.id().identifier().toString(), size, grid));
                        grids++;
                        if (rng.nextInt(3) == 0) {
                            List<ItemStack> broken = new ArrayList<>(grid);
                            broken.set(rng.nextInt(broken.size()), rng.nextBoolean() ? ItemStack.EMPTY : randomStack(rng, CRAFT_POOL));
                            w.println(craftRecord(h.id().identifier() + " (altered)", size, broken));
                            grids++;
                        }
                    }
                }
            }
            for (int k = 0; k < perRecipe * 200; k++) {
                int size = rng.nextBoolean() ? 3 : 2;
                List<ItemStack> grid = new ArrayList<>();
                double fill = rng.nextDouble();
                for (int i = 0; i < size * size; i++) grid.add(rng.nextDouble() < fill ? randomStack(rng, CRAFT_POOL) : ItemStack.EMPTY);
                w.println(craftRecord("random", size, grid));
                grids++;
            }
        }
        OUT.println("wrote " + grids + " grids to " + out);
    }

    static String craftRecord(String desc, int size, List<ItemStack> grid) {
        CraftingInput input = CraftingInput.of(size, size, grid);
        Optional<RecipeHolder<CraftingRecipe>> found = recipes.getRecipeFor(RecipeType.CRAFTING, input, LEVEL);
        StringBuilder b = new StringBuilder();
        b.append("{\"desc\": \"").append(desc.replace("\"", "'")).append("\", \"size\": ").append(size).append(", \"grid\": ").append(hexList(grid));
        if (found.isPresent()) {
            ItemStack result = found.get().value().assemble(input);
            b.append(", \"recipe\": \"").append(found.get().id().identifier()).append("\", \"result\": \"").append(hex(result))
                    .append("\", \"remaining\": ").append(hexList(found.get().value().getRemainingItems(input)));
        } else {
            b.append(", \"recipe\": null");
        }
        return b.append('}').toString();
    }

    static List<RecipeHolder<?>> sortedRecipes(Class<?> type) {
        List<RecipeHolder<?>> out = new ArrayList<>();
        for (RecipeHolder<?> h : recipes.getRecipes()) {
            if (type.isInstance(h.value())) out.add(h);
        }
        out.sort(java.util.Comparator.comparing(h -> h.id().identifier().toString()));
        return out;
    }

    /** Station inputs that some recipe takes: a stonecutter input, or smithing template/base/addition (sometimes one wrong). */
    static List<ItemStack> stationInput(String kind, Random rng) {
        List<ItemStack> g = new ArrayList<>();
        if (kind.equals("stonecutter")) {
            var list = sortedRecipes(net.minecraft.world.item.crafting.StonecutterRecipe.class);
            var r = (net.minecraft.world.item.crafting.StonecutterRecipe) list.get(rng.nextInt(list.size())).value();
            g.add(pick(r.input(), rng));
        } else {
            var list = sortedRecipes(net.minecraft.world.item.crafting.SmithingRecipe.class);
            var r = (net.minecraft.world.item.crafting.SmithingRecipe) list.get(rng.nextInt(list.size())).value();
            g.add(r.templateIngredient().map(i -> pick(i, rng)).orElse(ItemStack.EMPTY));
            g.add(pick(r.baseIngredient(), rng));
            g.add(r.additionIngredient().map(i -> pick(i, rng)).orElse(ItemStack.EMPTY));
            if (rng.nextInt(5) == 0) g.set(rng.nextInt(3), ItemStack.EMPTY);
            if (rng.nextInt(6) == 0) g.set(1, randomStack(rng, SMITH_POOL));
        }
        for (ItemStack st : g) {
            if (!st.isEmpty() && rng.nextInt(3) > 0) st.setCount(1 + rng.nextInt(st.getMaxStackSize()));
        }
        return g;
    }

    static ItemStack pick(Ingredient ing, Random rng) {
        List<Holder<net.minecraft.world.item.Item>> items = ing.items().toList();
        return new ItemStack(items.get(rng.nextInt(items.size())), 1);
    }

    static List<ItemStack> emptyGrid(int size) {
        List<ItemStack> g = new ArrayList<>();
        for (int i = 0; i < size * size; i++) g.add(ItemStack.EMPTY);
        return g;
    }

    static List<ItemStack> gridFor(CraftingRecipe recipe, String id, int size, Random rng) {
        List<ItemStack> g = emptyGrid(size);
        List<ItemStack> special = specialGrid(recipe, id, size, rng);
        if (special != null) return special;
        if (recipe instanceof ShapedRecipe shaped) {
            int w = shaped.getWidth(), h = shaped.getHeight();
            if (w > size || h > size) return null;
            int ox = rng.nextInt(size - w + 1), oy = rng.nextInt(size - h + 1);
            boolean mirror = rng.nextBoolean();
            var ings = shaped.getIngredients();
            for (int y = 0; y < h; y++) {
                for (int x = 0; x < w; x++) {
                    var ing = ings.get((mirror ? w - 1 - x : x) + y * w);
                    if (ing.isPresent()) g.set(ox + x + (oy + y) * size, pick(ing.get(), rng));
                }
            }
            return g;
        }
        var placement = recipe.placementInfo();
        if (!placement.isImpossibleToPlace()) {
            List<Ingredient> ings = new ArrayList<>(placement.ingredients());
            if (recipe instanceof net.minecraft.world.item.crafting.TransmuteRecipe) {
                int keep = 2 + rng.nextInt(ings.size() - 1);
                ings = ings.subList(0, Math.min(keep, ings.size()));
            }
            if (ings.size() > size * size) return null;
            List<Integer> cells = new ArrayList<>();
            for (int i = 0; i < size * size; i++) cells.add(i);
            java.util.Collections.shuffle(cells, rng);
            for (int i = 0; i < ings.size(); i++) g.set(cells.get(i), pick(ings.get(i), rng));
            return g;
        }
        return null;
    }

    static List<RecipeHolder<?>> crafting;

    static List<RecipeHolder<?>> craftingRecipes() {
        if (crafting == null) {
            crafting = new ArrayList<>();
            for (RecipeHolder<?> h : recipes.getRecipes()) {
                if (h.value() instanceof CraftingRecipe && !h.id().identifier().getPath().contains("map")) crafting.add(h);
            }
        }
        return crafting;
    }

    static final String[] DYES = {"white_dye", "orange_dye", "red_dye", "blue_dye", "black_dye", "lime_dye", "purple_dye", "brown_dye"};

    static List<ItemStack> scatter(int size, Random rng, List<ItemStack> items) {
        if (items.size() > size * size) return null;
        List<ItemStack> g = emptyGrid(size);
        List<Integer> cells = new ArrayList<>();
        for (int i = 0; i < size * size; i++) cells.add(i);
        java.util.Collections.shuffle(cells, rng);
        for (int i = 0; i < items.size(); i++) g.set(cells.get(i), items.get(i));
        return g;
    }

    static List<ItemStack> specialGrid(CraftingRecipe recipe, String id, int size, Random rng) {
        String name = recipe.getClass().getSimpleName();
        List<ItemStack> items = new ArrayList<>();
        String color = DYES[rng.nextInt(DYES.length)].replace("_dye", "");
        switch (name) {
            case "BannerDuplicateRecipe" -> {
                int layers = 1 + rng.nextInt(7);
                StringBuilder pat = new StringBuilder();
                for (int i = 0; i < layers; i++) pat.append(i == 0 ? "" : ",").append("{pattern:\"minecraft:stripe_top\",color:\"red\"}");
                String own = id.replace("_banner_duplicate", "");
                items.add(parse(own + "_banner[banner_patterns=[" + pat + "]]", 1));
                items.add(parse((rng.nextInt(5) == 0 ? color : own) + "_banner", 1));
            }
            case "BookCloningRecipe" -> {
                items.add(parse("written_book[written_book_content={title:\"t\",author:\"a\",generation:" + rng.nextInt(4) + ",pages:[\"x\"]}]", 1));
                int n = 1 + rng.nextInt(8);
                for (int i = 0; i < n; i++) items.add(parse("writable_book", 1));
            }
            case "DecoratedPotRecipe" -> {
                if (size < 3) return null;
                String[] sides = {"brick", "angler_pottery_sherd", "archer_pottery_sherd", "danger_pottery_sherd"};
                List<ItemStack> g = emptyGrid(3);
                for (int c : new int[] {1, 3, 5, 7}) g.set(c, parse(sides[rng.nextInt(sides.length)], 1));
                return g;
            }
            case "DyeRecipe" -> {
                String[] armor = {"leather_chestplate", "leather_helmet[dyed_color=65280]", "leather_boots", "wolf_armor"};
                String own = id.replace("_dyed", "");
                items.add(parse(rng.nextInt(3) > 0 ? own + (rng.nextBoolean() ? "[dyed_color=4660]" : "") : armor[rng.nextInt(armor.length)], 1));
                int n = 1 + rng.nextInt(Math.min(8, size * size - 1));
                for (int i = 0; i < n; i++) items.add(parse(DYES[rng.nextInt(DYES.length)], 1));
            }
            case "FireworkRocketRecipe" -> {
                items.add(parse("paper", 1));
                int f = 1 + rng.nextInt(4);
                for (int i = 0; i < f; i++) items.add(parse("gunpowder", 1));
                int st = rng.nextInt(4);
                for (int i = 0; i < st; i++) items.add(parse("firework_star[firework_explosion={shape:\"burst\",colors:[I;" + rng.nextInt(99999) + "],has_trail:true}]", 1));
            }
            case "FireworkStarRecipe" -> {
                items.add(parse("gunpowder", 1));
                int n = 1 + rng.nextInt(3);
                for (int i = 0; i < n; i++) items.add(parse(DYES[rng.nextInt(DYES.length)], 1));
                String[] extra = {"fire_charge", "gold_nugget", "creeper_head", "feather", "diamond", "glowstone_dust"};
                int e = rng.nextInt(3);
                for (int i = 0; i < e; i++) items.add(parse(extra[rng.nextInt(extra.length)], 1));
            }
            case "FireworkStarFadeRecipe" -> {
                items.add(parse(rng.nextInt(4) == 0 ? "firework_star" : "firework_star[firework_explosion={shape:\"star\",colors:[I;1],fade_colors:[I;5]}]", 1));
                int n = 1 + rng.nextInt(4);
                for (int i = 0; i < n; i++) items.add(parse(DYES[rng.nextInt(DYES.length)], 1));
            }
            case "ImbueRecipe" -> {
                if (size < 3) return null;
                List<ItemStack> g = emptyGrid(3);
                for (int i = 0; i < 9; i++) g.set(i, parse("arrow", 1));
                String[] center = {"lingering_potion[potion_contents={potion:\"minecraft:swiftness\"}]", "lingering_potion", "lingering_potion[!potion_contents]"};
                g.set(4, parse(center[rng.nextInt(center.length)], 1));
                return g;
            }
            case "MapExtendingRecipe" -> {
                if (size < 3) return null;
                List<ItemStack> g = emptyGrid(3);
                for (int i = 0; i < 9; i++) g.set(i, parse("paper", 1));
                g.set(4, parse("filled_map[map_id=" + rng.nextInt(7) + "]", 1));
                return g;
            }
            case "RepairItemRecipe" -> {
                String[] tools = {"iron_pickaxe", "diamond_sword", "wooden_axe", "enchanted_book"};
                String t = tools[rng.nextInt(tools.length)];
                String[] enchant = {"", ",enchantments={binding_curse:1}", ",enchantments={sharpness:2,vanishing_curse:1}", ",enchantments={unbreaking:3}"};
                items.add(parse(t + "[damage=" + rng.nextInt(200) + enchant[rng.nextInt(enchant.length)] + "]", 1));
                items.add(parse(t + "[damage=" + rng.nextInt(200) + enchant[rng.nextInt(enchant.length)] + "]", 1));
            }
            case "ShieldDecorationRecipe" -> {
                items.add(parse(rng.nextInt(5) == 0 ? "shield[banner_patterns=[{pattern:\"minecraft:cross\",color:\"blue\"}]]" : "shield", 1));
                items.add(parse(color + "_banner" + (rng.nextBoolean() ? "[banner_patterns=[{pattern:\"minecraft:cross\",color:\"blue\"}]]" : ""), 1));
            }
            default -> {
                return null;
            }
        }
        return scatter(size, rng, items);
    }
}
