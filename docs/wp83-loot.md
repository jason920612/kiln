# wp83-loot:模擬端沒呼叫(或寫死結果)的 loot table

分支 `wp83-loot`(從 `wp83-datapacks` `d14de992` 開出,只推到 `vm`)。目標:datapack 改了某張 loot table,遊戲就跟著改 ——
模擬端要在原版呼叫的地方、用原版的 loot context(參數)去 roll 那張表。表引擎(`kiln-loot`)本來就會算所有表,缺的是「呼叫」。
做法:每一個呼叫先用 `tools/jpv.py` 讀原版位元碼(`Block.dropFromBlockInteractLootTable`、`BlockTransformer.transformBlock`、`Creeper.killedEntity`、
`EquipmentUser.equip`、`LivingEntity.dropFromGiftLootTable` / `dropFromShearingLootTable` / `dropFromLootTable`),再照抄參數、亂數抽取順序、掉落位置與速度、聲音與遊戲事件;
每個呼叫一個用「自己寫的小 datapack」的測試(`crates/kiln-sim/tests/loot_calls.rs`,表的 JSON 寫在測試裡,沒有複製 Mojang 的檔案),
`tools/wp83/neg83_loot.sh` 把修正拿掉證明測試會失敗。

## 1. 這個套件新接上的呼叫

| 原版表 | 原版程式 | context(`LootContextParamSets`) | Kiln 的位置 | 備註 |
|---|---|---|---|---|
| `harvest/sweet_berry_bush` | `SweetBerryBushBlock.useWithoutItem`(年齡 > 1 才能採,採完回到 1) | `BLOCK_INTERACT`:`block_state`(採之前的狀態)、`origin`(方塊中心)、`interacting_entity`(玩家)、沒有 `tool` | `tools.rs` `block_use_without_item` → `harvest_berries` | 以前 Kiln **完全不能右鍵採甜莓**(只有狐狸吃、蜜蜂授粉);音高 `0.8 + nextFloat*0.4`,`setBlock(..., 2)` 加 `block_change` 遊戲事件 |
| `harvest/cave_vine` | `CaveVines.use`(藤與藤頭兩種方塊,`berries=true`) | 同上 | 同上 | 音高 `Mth.randomBetween(random, 0.8, 1.2)` |
| `harvest/beehive` | `BeehiveBlock.dropHoneycomb`(玩家的剪刀、發射器的剪刀) | `BLOCK_INTERACT`:`tool` = 剪刀;玩家用才有 `interacting_entity`,發射器沒有 | `beehive.rs` `drop_honeycombs_into` / `drop_honeycombs`(發射器) | 以前寫死 3 個蜂窩 |
| `carve/pumpkin` | `PumpkinBlock.useItemOn`(剪刀) | `BLOCK_INTERACT`:`tool` = 剪刀、`interacting_entity` = 玩家、`block_state` = 南瓜 | `tools.rs` `carve` | 以前寫死 4 顆種子;位置與速度照 lambda(`+0.65*step`、`0.05*step + nextDouble*0.02`);補上 `shear` 遊戲事件,`USED` 統計記的是剪刀(以前剪刀剛好壞掉會記成空氣) |
| `till/rooted_dirt` | `BlockTransformer.transformBlock`(hoe 的第二條規則,`loot` + `drop_strategy: clicked_face`) | `BLOCK_INTERACT`:`tool` = 鋤頭、`interacting_entity` = 玩家、`block_state` = 鋤之前的狀態 | `tools.rs` `till` → `interact_loot::pop_resource_from_face` | 以前寫死 1 個懸根;`popResourceFromFace` 的位置與速度照抄(以前速度是雜湊亂數) |
| `charged_creeper/root`(及 5 張子表) | `Creeper.killedEntity`:`shouldDropLoot` ∧ `isPowered` ∧ 還沒掉過頭 → 受害者 `dropFromLootTable(level, source, false, CHARGED_CREEPER, 掉在受害者身上)`,有東西掉才設 `droppedSkulls` | `ENTITY`:`this_entity` = **受害者**(表用它的類型挑頭)、`origin`、`damage_source` = 爆炸(攻擊者 = 苦力怕);沒有 `last_damage_player` | `kiln-entity` `mob::creeper_blast`(爆炸期間把苦力怕登記成 killer)、`killer_notified` → `EntityLevel::charged_creeper_loot`(sim 實作)→ `Event::ItemDrops` | 以前被充能苦力怕炸死的怪**完全不掉頭**;一隻苦力怕一顆(第一個「表有東西可給」的受害者) |
| `equipment/*`(`trial_chamber`、`_melee`、`_ranged`,以及任何 spawn data 的 `equipment`) | `TrialSpawner.spawnMob` / `BaseSpawner.serverTick`:finalize(與 `setPersistenceRequired`)之後 `spawnData.getEquipment().ifPresent(mob::equip)` → `EquipmentUser.equip`(`resolveSlot`:`equippable` 的槽或主手、每槽一次、盔甲一件;`slot_drop_chances` 設掉落機率) | `EQUIPMENT`:`this_entity`、`origin`(沒有 damage source) | `kiln-entity` `mob/spawn_equipment.rs`;`trial_spawner.rs` 與 `spawner.rs` 呼叫;`EntityLevel::equipment_loot`(sim 實作) | 以前 spawn data 的 `equipment` 只保存不 roll(`trial_spawner.rs` 檔頭寫著缺口):不祥試煉怪沒有裝備 |
| `gameplay/hero_of_the_village/*` | `GiveGiftToHero.throwGift` → `dropFromGiftLootTable` + `BehaviorUtils.throwItem(villager, stack, hero.position())` | `GIFT`:`this_entity`、`origin` = 村民位置 | `villager.rs` `throw_gift` → `Event::GiftThrow` | 表本來就有 roll,但物品是**掉在村民腳邊**;現在從村民眼睛下 0.3 格朝英雄丟,每刻 0.3 格 |
| `chests/jungle_temple_dispenser` | `JungleTemplePiece`(兩個發射器) | – | `worldgen/structure/legacy/scattered.rs` | worldgen 用的 id 是舊名 `dispensers/jungle_temple`(26.3 沒有這張表),叢林神廟的兩個發射器**一直是空的**;改成正確的 id,並加 `loot_table_names.rs` 守門 |

### 既有 gift / shearing 呼叫的 context 補正(`entities.rs` `loot_drop`)

- `GIFT`、`SHEARING`、`EQUIPMENT` 不是死亡:`damage_source` 參數不存在(`DeathExtra.no_damage_source`),`damage_source_properties` 之類的條件要失敗(以前恆為真)。
- `origin` 是實體的位置(`this.position()`),不再是掉落點(貓的禮物掉在床邊、村民的禮物…)。
- 剪毛的 `tool` 參數 = 剪刀(`DeathExtra.tool`、`LootContext::tool`),`match_tool` 條件成立;玩家剪羊(`entities.rs` 的 `out.shear` 路徑)用手上的剪刀。

## 2. 共用的新零件(`kiln-sim/src/interact_loot.rs`)

- `block_interact(level, 表, pos, 狀態, tool, 有無玩家, salt)`:`Block.dropFromBlockInteractLootTable` 的 roll(沒有任何 loot 資料時回 `None`,呼叫端給「沒有 datapack 時」的舊結果;有資料但沒這張表 = 空表,跟原版一樣)。
- `pop_resource`、`pop_resource_from_face`:`Block.popResource` / `popResourceFromFace`(位置、速度、`block_drops` 規則、撿取延遲 10)。
- 亂數:原版用伺服器全域的 `RandomSequences`(`random_sequence` 欄位),分區平行的模擬沒辦法共用一條;改用「位置 + 遊戲刻」雜湊出的種子(跟方塊掉落同一個做法,I 類近似)。**位元不同,分布相同。**

## 3. 檢查過、本來就接好的呼叫(表 id → 位置)

| 表 | Kiln 位置 | 備註 |
|---|---|---|
| `blocks/*`、`entities/*`、`shearing/*`(羊、哞菇、雪傀儡、長滿苔蘚的骷髏) | `blocks.rs` `block_drops`、`mob/mod.rs` `die`、各 `kinds/*` 的 `Event::ShearLoot` | 依方塊名 / 實體類型取表 |
| `gameplay/cat_morning_gift`、`chicken_lay`、`panda_sneeze`、`sniffer_digging`、`turtle_grow`、`armadillo_shed`、`piglin_bartering` | `Event::GiftLoot` | `cat_gift.rs`、`animals.rs` 有測試;貓 / 村民 / 豬布林的掉落位置與丟法各自照原版 |
| `brush/armadillo` | `armadillo.rs` | context 近似(見 §4) |
| `gameplay/fishing`(+ `fish` `junk` `treasure`) | `fishing.rs` | 子表是 `loot_table` entry,表引擎處理 |
| `archaeology/*`(沙漠神殿、海底遺跡、古蹟廢墟) | 方塊實體的 `LootTable`(worldgen 寫、`brush.rs` `unpack_loot` 刷時 roll) | `ARCHAEOLOGY` context(origin、this_entity、tool) |
| `chests/*`、`barrels/*`、`pots/*`、`dispensers/trial_chambers/*` | 方塊實體 / 載具的 `LootTable`(結構模板的 NBT 或 worldgen 常數),`container/mod.rs` `unpack_loot` | `loot_table_names.rs` 檢查 worldgen 寫的 id 都存在 |
| `chests/trial_chambers/reward_*`、金庫的 `override_loot_table_to_display` | `vault.rs`(`VAULT` context) | 預設表是常數,datapack 的 `loot_table` 欄位照用 |
| `spawners/trial_chamber/*`、`spawners/ominous/*` | `trial_spawner.rs` + `entities.rs` `trial_eject` / `trial_dispensing_items`(`EMPTY` context) | 設定從 datapack 的 `trial_spawner/*.json`(`mob_spawner.rs`)讀,預設值是常數 |

## 4. 還寫死 / 近似的(以及原因)

- **hoe 的方塊轉換規則**:原版 26.3 的 `block_transformer`(`hoe`、`axe`、`shovel`)是 datapack registry,`loot` 與 `drop_strategy` 在資料裡。Kiln 只把 `till/rooted_dirt` 接上,轉換清單(草→農地、粗泥→泥…)與 `drop_strategy` 還是寫在 `tools.rs`;資料 registry 的部分屬於 wp83-datapacks。
- **亂數序列**:見 §2。
- **`brush/armadillo`**(`ENTITY_INTERACT`:`target_entity`、`interacting_entity`、`tool`)與 **`gameplay/piglin_bartering`**(`PIGLIN_BARTER`:只有 `this_entity`)走 `GiftLoot`,context 是 `this_entity` + `origin`;表用到 `tool` / `interacting_entity` 的 datapack 在這兩處條件會失敗。
- **充能苦力怕殺死的玩家**:原版 `Creeper.killedEntity` 對任何生物都跑,玩家死亡走 `health.rs` 沒有接這個掛勾(原版 `charged_creeper/root` 對玩家也沒有項目,所以只有 datapack 加了玩家項目時才看得出來)。
- **狐狸吃甜莓 / 洞穴藤**:原版 `Fox` 自己算數量(不走表),Kiln 同。
- **沙漠水井**:Kiln 的 worldgen **沒有 `desert_well` 地物**,`archaeology/desert_well` 沒有地方呼叫(地物做出來時用 `place_suspicious_sand` 同一個做法);古蹟廢墟的 `archaeology/trail_ruins_*` 由 `append_loot` 處理器寫進方塊實體。
- **村民禮物 / 豬布林交易**:物品的丟法(起點、速度)照 `BehaviorUtils.throwItem`,但物品實體的隨機部分(`ItemEntity` 的初速亂數)用雜湊而不是實體自己的亂數。

## 5. 驗證

- `crates/kiln-sim/tests/loot_calls.rs`(10 個測試,用 `datapacks/p83` 寫入自己的小表並 `reload` + `datapack enable`):南瓜(剪刀作為 tool)、甜莓(成熟與否由 `block_state` 決定,1 階不採)、洞穴藤與藤頭、蜂窩、鋤根土、充能苦力怕(一隻一顆、未充能不掉)、生怪磚的裝備(`slot_drop_chances` 也跟)、村民禮物、剪羊(tool = 剪刀)。
- `crates/kiln-sim/tests/loot_table_names.rs`:所有原始碼裡寫的 loot table id 都存在於 vanilla 資料(抓到 `dispensers/jungle_temple`)。
- `tools/wp83/neg83_loot.sh`:每個修正拿掉,對應測試必須失敗(結果見下)。

### 結果(VM,`CARGO_BUILD_JOBS=3 RUST_TEST_THREADS=4`)

- `cargo check --workspace --all-targets`:沒有警告。
- `cargo test --release -p kiln-sim -p kiln-entity -p kiln-worldgen --no-fail-fast`(`KILN_DATAPACK=/home/test/kiln/work/generated`)與 `-p kiln-sim`(不設 `KILN_DATAPACK`,loot 測試略過):全部通過,含 `determinism.rs`(6 個)、`crowd_golden.rs`(4 個)、`cat_gift.rs`。
- `tools/wp83/neg83_loot.sh`:9 項「修正拿掉測試就失敗」全部如預期(南瓜、甜莓、洞穴藤、鋤根土、蜂巢、充能苦力怕、生怪磚裝備、剪毛的 `tool`、叢林神廟發射器的表 id);**沒有**對應負向檢查的:村民禮物測試(`a_villager_throws_the_hero_gift_its_table_rolls`)—— 那張表以前就有 roll,這個測試只守住「datapack 改表會生效」,丟向英雄的速度與起點沒有斷言。
