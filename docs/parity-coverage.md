# Kiln 與原版 26.3 的一致性涵蓋矩陣（wp44 稽核）

基準：`main` 的 72ef868（本文件隨 wp44 分支更新，「wp44 之後」一欄與第 6 節記錄本工作包的改動）。
稽核範圍是玩家能觀察到的行為。方法：先問「完整驗證」要涵蓋什麼，刪掉不可觀察的工作，不另建新框架；只有稽核指出的洞才補。

## 0. 「完整驗證」要涵蓋什麼

一個生存玩家（含 AFK 農場、紅石、刷怪塔、伺服器管理者）能觀察到的東西，分成這些面向，每個面向都要有可重跑的證據：

1. 世界：地形、生物群系、結構、特徵（逐方塊）、光照、出生點。
2. 方塊自己做的事：方塊更新與排程 tick、隨機 tick（作物、草、藤蔓、冰、銅、樹葉…）、流體、紅石、活塞、火、掉落式方塊、傳送門。
3. 方塊實體與容器：熔爐、漏斗、釀造台、發射器、信標、選單點擊、合成與配方、戰利品。
4. 物品與使用：吃喝、工具、弓弩、桶、骨粉、打火石、剪刀、書與告示牌。
5. 戰鬥、傷害、效果、附魔。
6. 玩家：移動檢查、飢餓、經驗、死亡重生、進度、統計、睡眠。
7. 生物：AI、生成規則（自然、刷怪磚、結構）、掉落、繁殖與馴服、轉換、騎乘。
8. 其他實體：投射物、物品、經驗球、載具、TNT、掉落方塊、雲、煙火、畫與展示框。
9. 世界狀態：天氣、時間、睡眠、襲擊、巡邏、商人、村莊、維度與傳送門、終界戰、遊戲規則、世界邊界。
10. 指令、datapack、loot、predicate。
11. 協定（封包位元組）與持久化（Anvil 往返、原版載入 Kiln 的存檔）。

證據分級（本文件全程使用）：

| 級別 | 意義 |
|---|---|
| **A** | 原版錄製的逐位元向量（原版伺服器在行程內跑場景，Kiln 重播並逐項比對），或原版載入 Kiln 寫出的檔案。 |
| **A\*** | 只有部分路徑有向量。 |
| **B** | 用別的方法與原版伺服器或真 client 比對（`command_diff.py` 雙伺服器、`*_view.py` 真 client 截圖、`persist_check.py`）。 |
| **C** | 只有 Kiln 自己的單元／模擬測試（寫的人對原版的理解，沒有原版當裁判）。 |
| **D** | 未實作、部分實作或沒有驗證。 |

## 1. 摘要

| 區域 | 項目數 | A | A\* | B | C | D | 備註 |
|---|---|---|---|---|---|---|---|
| 方塊（有行為的 vanilla 方塊類別，`AuditBlockBehaviour.java` 盤點） | 218 類 | 57 | – | 2 | 58 | **101** | 隨機 tick 幾乎全缺（第 3.1 節） |
| 方塊實體、容器、選單 | 48 | 14 | – | 0 | 16 | 18 | |
| 合成與配方 | 14 | 10 | – | 1 | 3 | 0 | |
| 物品欄點擊與同步 | 16 | 12 | – | 0 | 1 | 3 | |
| 戰利品 | 17 | 14 | – | 0 | 2 | 1 | |
| datapack、function、tag、predicate | 15 | 0 | – | 9 | 5 | 1 | |
| 物品元件 | 9 | 6 | – | 0 | 2 | 1 | |
| 生物（26.3 的 Mob 子類 90 種） | 90 | 83（其中 57 另有真 client 渲染檢查） | – | – | 0 | **7 種未實作** | 另有約 20 類已實作但有功能缺口 |
| 非生物實體 | 32 | 13 | 8 | 1 | 8 | 2 | |
| 戰鬥與傷害 | 25 | 13 | 1 | 0 | 7 | 4 | |
| 效果與藥水 | 8 | 5 | 0 | 0 | 2 | 1 | |
| 附魔（43 種） | 43 | 27 | 0 | 0 | 12 | 4 | |
| 物品與使用 | 30 | 7 | 8 | 0 | 6 | 9 | |
| 玩家系統 | 16 | 3 | 3 | 5 | 4 | 1 | |
| 世界生成 | 30 | 20 | – | 2 | 3 | 5 | 地形與特徵是專案最紮實的部分 |
| 天氣、襲擊、維度、規則 | 32 | 8 | – | 0 | 20 | 4 | |
| 指令（92 條） | 92 | 72 | – | 0 | 20 | 0 | 缺口在子功能 |
| 協定（封包） | 213 play + 47 其他 | 118 + 25 | – | 57（B／C 混合）+ 15 | 4 | 32 + 3 | |
| 持久化 | 37 | 11 | – | 0 | 15 | 11 | |

讀法：A 欄多的區域（世界生成、容器點擊、loot、指令、生物 AI）可以信賴；C、D 欄多的區域（方塊自驅行為、玩家環境傷害、結構生怪、書與告示牌、地圖、部分選單）就是缺口。

## 2. 現有 parity 套件與本次實際執行結果

環境：`KILN_WORK` 指向有 vanilla 資料與錄製向量的 work 目錄，`KILN_DATAPACK` 指向 `work/generated`，release 建置。數字是 cargo 與測試自己印的。

| 套件（指令） | 比對對象 | 結果（main 72ef868） |
|---|---|---|
| `cargo test --workspace --release`，不設環境變數 | 全部單元與整合測試（parity 測試靜默略過） | 1,068 通過、0 失敗、11 ignored（137 個測試行程） |
| 同上，設 `KILN_WORK`＋`KILN_DATAPACK` | 同上，有 datapack 的測試實際執行 | 1,068 通過、0 失敗、11 ignored |
| `mob_parity`（`work/m6-mobs2/vectors.jsonl`） | 生物逐 tick 的位置、速度、旋轉、health、目標、運行中的 goal | **867/867 scenario、400,824 個生物狀態相同** |
| `mob_parity`（`wp34/mob_spear`、`wp36/mob_kills`、`wp36/mob_ench`） | 矛、殭屍殺村民、附魔矛 | 17／22／16 scenario 全過（8,800／3,972／4,800 狀態相同） |
| `finalize_parity`（`wp33/finalize.jsonl`、`wp34/finalize_hard.jsonl`） | 自然生成的 `finalizeSpawn`（裝備、騎乘者、附魔、level random 之後） | 31,900 筆相同、0 不同 |
| `entity_parity`（`wp4-entities/vectors.jsonl`，舊檔） | 物品、經驗球、TNT、掉落方塊、投射物等實體物理 | 951/951（現行 harness 約 1,201 個，存檔過期） |
| `fire_parity`（`wp-fire`） | 火的蔓延、燃燒、老化 | 7 scenario、270/270 輪 |
| `combat`（`wp34/combat/*`） | 戰鬥、riptide、馬物品欄、矛、附魔 helper | 戰鬥 114、riptide 110、矛 127、馬物品欄 240、附魔 helper 1,381 筆（damage 648、destroy_speed 280、durability 300、protection 45、armor_effectiveness 24、modifiers 72、knockback 12），0 失敗 |
| `effect_parity`（`wp9-effects`） | 效果、飢餓、溺水、火、飲食 | 131 scenario（5,908 tick）全過 |
| `container_parity`（`wp15-containers`、`wp36/containers`） | 漏斗、熔爐、比較器、唱片機 | 30 scenario（8,144 值）、14 scenario（1,400 值）全過 |
| `sculk_parity`（`m6s3-warden`） | sculk 感測器、催化劑、尖叫者 | 12 scenario、4,160/4,160 值 |
| `weather_parity`（`wx-weather`） | 天氣週期、天空暗度、降水、睡眠 | 674/674 |
| `item_parity`（`wp-itemuse`） | 視線射線（桶）、射擊、弩 | clip 1,983/2,000（互動形狀的面不同）、shoot 400/400、crossbow 69/300 逐位元（300 皆在 1e-6 內：JOML 以 float 旋轉） |
| `click_parity` 等（`KILN_PARITY=1`） | 物品欄點擊、合成、單一配方查詢、選單同步 | 31,000 序列、743,196 步、0 失敗（kiln-inventory 43 個測試） |
| `kiln-item`（`wp2-items/corpus.jsonl`） | 122 種物品元件 wire／NBT／hash／patch | 10,745 個物品堆、10,670 個元件值、整堆 64,444 通過（31 個測試） |
| `kiln-loot`（`KILN_PARITY=1`） | 戰利品表 | 36,336 case（1,445 張表）全過 |
| `kiln-worldgen`（`KILN_PARITY=1`，約 18 分鐘） | 密度函數、biome、地形、表面、洞穴、特徵、結構、高度、出生點 | 34 個測試全過；每個 seed 2,560 chunk、四層 0 不符；特徵 0 不符、0 略過 |
| `kiln-storage` | Anvil 往返、原生格式、原版世界讀取 | 39 個測試全過 |
| `kiln-proto` | 封包 golden、serverbound 向量 | 60 個測試全過 |
| `kiln-command` | 指令註冊與指令樹 | 119 個測試全過；92 條指令（2,470 個節點）與 `commands.json` 相符 |
| `region_stacks`、`determinism` | region 合併與分割、決定性（含多 worker） | 5／6 個測試全過 |
| `tools/blocks_diff.py`（原版伺服器現場） | 45 個方塊 scenario 的快照 | **45/45 scenario、1,125/1,125 快照相同** |

重要觀察：

- **預設 `cargo test` 在沒有 `work/` 時靜默略過所有 parity 測試**（測試直接 return，算「通過」）。本表的數字是設了環境變數才有的。CI 若要有意義，必須設 `KILN_WORK` 與 `KILN_PARITY=1`；沒設 `KILN_PARITY=1` 時 click 序列只跑前 300 個（共 31,000）、loot 每種只跑前 400 個 case。
- **`work/` 內有些錄製檔已過期**：`work/wp4-entities/vectors.jsonl` 只有 951 個 scenario，現行 `EntityVectors.java` 約產生 1,201 個（minecart、boat、firework、cart_*、arrow_vehicle 缺）；`work/m6-combat/vectors.jsonl` 49 個，最新的 `work/wp36/combat/vectors.jsonl` 114 個。已存在的 harness 重錄後與存檔逐位元相同（例：`FireVectors` 重錄 `cmp` 一致），所以過期只表示「測試沒打到新場景」。
- 所有 `tools/*Vectors.java` 現在讀 `KILN_HARNESS_PORT`，未設時自己找 25581–25583 中第一個空的埠。

## 3. 逐區域矩陣

### 3.1 方塊與方塊更新

驗證證據：

- `tools/blocks_diff.py`：原版伺服器凍結 tick 後逐 tick 步進，儲存區塊快照，Kiln 在 `TestLevel` 重播命令並逐方塊、逐排程 tick 比對。**45 個 scenario、1,125 個快照全部相同**（本次執行）。涵蓋連接形狀、彈出、水、岩漿、含水、紅石全套、活塞全套、鐵軌、樹葉距離。
- `FireVectors`（火，7 個 scenario，270 輪）、`SculkVectors`（12）、`ContainerVectors`（容器 44）、`WeatherVectors`（降水）。
- 新增：`BlockTickVectors.java`（wp44，隨機 tick 與排程 tick，第 6 節）。

分類以 `tools/AuditBlockBehaviour.java`（反射掃原版方塊登錄，列出每個方塊類別覆寫的 hook）加上 Kiln 原始碼的參照為準，共 218 個「有伺服器端行為」的方塊類別。**判讀：隨機 tick 在 `kiln_blocks::behaviour::random_tick` 只有樹葉與岩漿**，其餘都只是抽位置（`behaviour/mod.rs:299` 註解自己寫「Other random-tick behaviour (crop growth, grass spread, ...) is not simulated yet」）。

| 群組 | 級別 | 類別數 | 證據 | 缺口 |
|---|---|---|---|---|
| 連接形狀、支撐、掉落式彈出（柵欄、玻璃片、牆、樓梯、門與植物彈出） | A | 4 | blocks_diff 7 個 scenario（fence_connections、panes_and_bars、walls、stairs_shapes、snowy_grass、doors_and_plants、pop_offs），每個 25 個快照 |  |
| 流體（水、岩漿、含水方塊、流動與轉換） | A | 1 | blocks_diff 11 個 scenario（water_* 7、lava_* 4、waterlogged） | 氣泡柱 BubbleColumnBlock 不在內（見下） |
| 紅石元件：線、火把、中繼器、比較器、觀察者、紅石燈、音符盒、拉桿、按鈕、壓力板 | A | 12 | blocks_diff 14 個 scenario（wire_* 4、torch_*、repeater_* 3、comparators、observers、note_blocks、button_lever、lamps_doors、pressure_plates、leaves 之外） | 實體觸發的壓力板由實體測試保證（C） |
| 門、活板門、柵欄門 | A | 3 | blocks_diff doors_and_plants、trapdoors_gates、lamps_doors |  |
| 鐵軌（普通、動力、偵測） | A | 3 | blocks_diff rails、powered_rails | 偵測鐵軌被礦車觸發靠礦車實體（entity vectors A） |
| 活塞、黏液塊與蜂蜜塊推拉 | A | 5 | blocks_diff 8 個 scenario（pistons_push、piston_reactions、slime_honey、piston_qc_bud、piston_pulses、piston_redstone、piston_heads、piston_observers） | 活塞不帶實體（`blocks.rs:1078`） |
| 火 | A | 2 | FireVectors 7 個 scenario、270 輪 |  |
| 樹葉衰減 | A | 4 | blocks_diff leaves_distance；BlockTickVectors harness_leaves |  |
| 容器方塊與選單：箱子、陷阱箱、銅箱、木桶、終界箱、潛影盒、漏斗、熔爐類、合成台、切石機、鍛造台、唱片機、建立中的創生之心 | A | 16 | ContainerVectors 30＋14 個 scenario；click 向量 31,000 序列；CreakingVectors 10 | 開關狀態、opener 計數為 C；銅箱專屬測試 unverified |
| Sculk 系列 | A | 4 | SculkVectors 12 個 scenario |  |
| 魔法岩漿塊 | A | 1 | EffectVectors（hazard 向量） |  |
| TNT | A | 1 | blocks_diff tnt_in_water；EntityVectors 60 個點燃 TNT | 爆炸對生物的傷害為 C |
| 青蛙卵 | A | 1 | frog_tadpole 向量（產卵、孵化） |  |
| 傳送門與終界方塊（地獄門、終界門、終界閘門） | C | 3 | tests/dimensions.rs、dragon_fight.rs；B：dimensions_view.py | 沒有原版向量；終界傳送門框架 EndPortalFrameBlock 為 D |
| 掉落式方塊（沙、礫、混凝土粉、鐵砧、龍蛋） | C | 5 | 實體物理 101 個向量 A；方塊排程與落下條件為 kiln-blocks 單元測試 | 鐵砧傷害為 C |
| 床、重生錨 | C | 3 | tests/weather.rs、sleep.rs | 床爆炸、重生錨爆炸為 C |
| 鍋釜（空、水、熔岩、粉雪） | C | 4 | buckets.rs、tests/items.rs；降水填充 A（weather vectors） |  |
| 堆肥桶、蠟燭與蠟燭蛋糕、南瓜、鑿過的南瓜、乾草捆、發光方塊、小徑 | C | 8 | tools.rs、tests/items.rs、golem.rs | 蛋糕本體（吃蛋糕）為 D；乾草捆摔落減傷為 D（見玩家）；小徑 tick 轉泥土為 D |
| 骨粉可成長但無隨機 tick 的植物（草、蕨、海草、海泡菜、苔蘚、杜鵑、地獄真菌、玫瑰苔、花壇、灌木等） | C | 17 | tools.rs 骨粉 perform；tests/items.rs | 骨粉本身沒有向量；樹苗骨粉只進一階（樹不會長） |
| 發射器、投擲器 | C | 2 | tests/containers.rs；比較器輸出 A | 發射器只實作預設掉落、箭、雪球、蛋、水與岩漿桶、礦車（`dispense.rs:6`）；點擊選單 A |
| 信標、附魔台、釀造台、砂輪、織布機、鐵砧選單 | C | 5 | tests/containers.rs、kiln-inventory/tests/menus.rs | 選單點擊無向量（見容器區） |
| 頭顱（凋零骷髏頭、玩家頭、豬布林頭等） | C | 7 | wither.rs（凋零建造） | 玩家頭顱 profile 放置時不套用 |
| 避雷針 | C | 1 | weather.rs 單元測試 | 氧化版為 D |
| 蜘蛛網、蓮葉 | C | 2 | mob/effects.rs、behaviour/support.rs | 玩家被蛛網減速為 D |
| 史萊姆卵（嗅探獸蛋） | C | 1 | sniffer 向量（挖掘、繁殖）；孵化為 kiln-blocks 單元 |  |
| 測試方塊（GameTest） | B | 2 | command_diff.py test 102 行；tests/gametest.rs |  |
| 農田與作物（農田、小麥、胡蘿蔔、馬鈴薯、甜菜、火炬花、瓶子草、地獄疙瘩、南瓜與西瓜莖、可可、甜莓叢、甘蔗、仙人掌、竹子與竹筍） | D | 15 | 隨機 tick 完全未實作（`behaviour/mod.rs:299`）；wp44-farming 進行中 | 生長、乾涸、折斷 |
| 草蔓延與死亡、菌絲、地獄菌毯、樹苗（不會長成樹） | D | 5 | 隨機 tick 未實作；wp44-growth 進行中（樹苗另案） | 草不蔓延、樹苗不長樹 |
| 藤蔓類與昆布（昆布、垂泣藤、扭曲藤、洞穴藤、藤蔓、發光地衣、紫頌花） | D | 13 | 隨機 tick 未實作；昆布只有 updateShape；wp44-growth 進行中 | 不生長 |
| 融冰、融雪、霜冰 | D | 3 | 降水形成冰與雪為 A（天氣向量）；融化的隨機 tick 未實作；wp44-growth 進行中 | 冰雪不融化 |
| 銅氧化與其他風化銅 | D | 14 | 隨機 tick 未實作；wp44-misc 進行中 | 銅不氧化 |
| 海龜蛋、紅石礦、紫水晶、滴水石、乾燥的哈氣、硫磺 | D | 9 | 隨機 tick 未實作；wp44-misc 進行中 |  |
| 珊瑚、海綿、鷹架、絆線與鉤、目標方塊、大型垂葉 | D | 13 | 排程 tick／鄰居更新未實作；wp44-growth／misc 進行中 | 珊瑚不會死亡、海綿不吸水、鷹架不塌 |
| 刷怪磚、試煉刷怪磚、寶庫 | D | 3 | 只存 NBT；wp44-spawner 處理刷怪磚 | 不生怪 |
| 告示牌、懸掛式告示牌 | D | 4 | 只存 NBT；`SignUpdate` 被丟棄；wp44-interact 進行中 | 不能編輯、染色、上蠟 |
| 營火、蜂巢、鐘、講台、裝飾陶罐、書架、擱板、合成器、製圖台、潛影導管 | D | 10 | 只存 NBT 或選單不完整 | 營火不烹飪、蜂巢無蜜蜂、鐘不響、講台無選單 |
| 指令方塊、結構方塊、拼圖方塊 | D | 3 | 只存 NBT；封包被丟棄 |  |
| 其他：日光感測器、旗幟、花盆、蛋糕、凋零玫瑰、終界傳送門框架、氣泡柱、可摔落的刷子方塊 | D | 9 | 未實作 | 日光感測器無輸出、蛋糕不能吃、凋零玫瑰無效果、無法放眼進框架 |

沒列的純形狀類別（樓梯、牆、鐵欄杆等，無伺服器端 hook）由 `blocks_diff` 的連接與彈出 scenario 一併驗證（A）。

### 3.2 方塊實體、容器、選單、配方、戰利品、datapack

| 項目 | 級別 | A 數量 | 依據 | 缺口 |
|---|---|---|---|---|
| 熔爐／高爐／煙燻爐 | A | 7 個容器 scenario＋約 6,300 個 click 序列 | `container_parity.rs`、`click_parity.rs` | 經驗與 `recipes_used` 為 C；經驗球合併未模擬 |
| 漏斗 | A | 9 | 同上 | 對釀造台、堆肥桶、裝飾陶罐、合成器無向量 |
| 容器比較器輸出 | A | 14 | `comparator_*` | – |
| 箱子、陷阱箱、大箱、木桶、終界箱、潛影盒（選單） | A | click 序列各約 2,100 | `click_parity.rs`；`containers_view.py`（B） | 開關音效、opener、蓋子遮擋為 C |
| 唱片機 | A | 14 | `wp36/containers` | 內嵌歌曲不播 |
| 合成台、切石機、鍛造台選單 | A | 4,277／686／669 序列 | `click_parity.rs`、`crafting_parity.rs` | 配方結果逐配方比對僅隨機覆蓋 |
| 馬、驢、騾、骷髏馬物品欄 | A | 240 | `mount.jsonl` | 無羊駝向量 |
| Sculk 感測器、創生之心 | A | 12／10 | `sculk_parity.rs`、`tests/creaking.rs` | – |
| 釀造台運作 | C | 0（配方查詢 A 3,289 筆） | `tests/containers.rs` | 計時、燃料、選單點擊無向量 |
| 發射器 | C（部分 D） | 2 | `tests/containers.rs` | 只實作預設掉落、箭、雪球、蛋、水與岩漿桶、空桶、礦車；缺火焰彈、骨粉、盔甲、藥水、剪刀、TNT、煙火、船、潛影盒等（`dispense.rs:6`） |
| 投擲器、終界箱、信標、附魔台、鐵砧、砂輪、織布機、商人選單、羊駝物品欄、床、重生錨 | C | 0 | `tests/*`、`kiln-inventory/tests/menus.rs` | 選單點擊向量只涵蓋 13 種選單（見下） |
| 合成器、講台、製圖台、裝飾陶罐、書架、營火、潛影導管、鐘、蜂巢 | D | 0 | 只有 NBT 往返（B） | 見 3.1 |
| 刷怪磚、試煉刷怪磚、寶庫 | D | 0 | – | 不運作（wp44-spawner 處理刷怪磚） |
| 告示牌編輯、書、橫幅與頭顱放置資料 | D | 0 | `SignUpdate`、`EditBook` 被丟棄（`region.rs:1102`） | wp44-interact 處理 |
| 合成配方：有形、無形、轉換、特殊、冶煉、高爐、煙燻、營火烹飪、釀造、切石、鍛造 | A | 827／375／33＋全部特殊配方；冶煉類 116 配方；釀造 279 | `crafting_parity.rs`、`single_parity.rs` | 有 4 個配方沒被向量打到 |
| 配方書顯示與放置 | C | 0 | `recipe_book.rs`、`menus.rs` | 無封包位元組比對 |
| 物品欄點擊（PICKUP、QUICK_MOVE、PICKUP_ALL、QUICK_CRAFT、SWAP、THROW、CLONE、創造槽、按鈕、關閉、無效封包） | A | 約 74 萬步／31,000 序列 | `click_parity.rs` | 向量只涵蓋 13 種選單，缺鐵砧、砂輪、附魔台、織布機、製圖台、釀造台、信標、商人、講台、合成器 |
| 選單同步（state id、過期預測、竄改預測） | A | 31,000 序列 | `sync_parity.rs` | – |
| Pick Block、Bundle 選取、`ContainerSlotStateChanged` | D | 0 | 封包被丟棄 | wp44-interact |
| 戰利品：方塊、方塊互動、實體、箱子、釣魚、贈禮、考古、豬布林、剪毛、裝備 | A | 36,336 case／1,445 張表（原版共 1,447） | `vanilla_parity.rs` | `blocks/beacon`、`entities/pillager` 無向量 |
| 戰利品函數 42 種、條件 20 種、entry 9 種 | A | 全部實作；有 4 個條件向量沒用到 | `kiln-loot` | `exploration_map` 與 dynamic entry 在 sim 沒實作 |
| 戰利品接線：收成（甜莓、洞穴藤、蜂巢）、刮南瓜、鋤根土、貓晨間禮物、苦力怕充能、試煉箱子、刷子 | D | 0（表本身 A） | – | 表引擎 A，但模擬端沒呼叫或寫死 |
| datapack：pack 探索與啟用、function、macro、schedule、predicate、item modifier、dialog、gametest、`/place`、`/reload` | B | 0 | `command_diff.py` 1,937 個比較單位 | – |
| 遊戲邏輯用的 tag | C | 0 | `tags.rs` | 寫死的原版內建表，datapack 改 tag 對合成、挖掘、信標無效 |
| 自訂 registry（附魔、傷害類型、畫、橫幅圖案、歌曲、裝飾、dialog） | D | 0 | – | 登入送的 registry 只有條目名，datapack 自訂條目不同步 |
| 物品元件 122 種 wire／NBT／hash／patch 位元組往返 | A | 10,745 個物品堆 | `vanilla_corpus.rs` | 部分元件的向量值少於 6 個；預設元件對 `item_components.json` 逐項比對為 C |

### 3.3 物品、戰鬥、效果、附魔、玩家

| 項目 | 級別 | A 數量 | 依據 | 缺口 |
|---|---|---|---|---|
| 玩家對玩家近戰、護甲、吸收、受傷冷卻、擊退、荊棘 | A | 114 個戰鬥 scenario＋helper 約 380 筆 | `combat_parity.rs`、`enchant_parity.rs` | – |
| **玩家對生物近戰** | C | 0 | `entities.rs:2187` | CombatVectors 只有玩家目標（wp44-combat） |
| **橫掃攻擊對生物** | D | 0 | `combat.rs:837-1000` | 只掃玩家（wp44-combat） |
| **重錘（smash、density、wind burst）** | D | `mace_standing` 1 | – | `smash_damage_per_fallen_block` 模擬從不呼叫（wp44-combat） |
| 矛（玩家與生物） | A | 127＋17 | `spear_parity.rs` | – |
| 三叉戟 riptide | A | 110 | `combat_parity.rs` | – |
| 盾牌、不死圖騰 | C | 0 | `shield.rs`、`tests/health.rs` | – |
| **摔落傷害** | C（簡化） | 5 筆 helper | `health.rs:1073` | 固定 `(fell−3).floor()`：無 safe_fall_distance 屬性、無 fall_damage_multiplier、無落地方塊效果（乾草 0.2、床 0.5、史萊姆、蜂蜜、蜘蛛網、梯子、粉雪）、鞘翅撞牆（wp44-player） |
| 溺水、火、岩漿、岩漿塊、營火、飢餓與餓死 | A | 11＋17＋3 | `effect_parity.rs` | – |
| **仙人掌、甜莓叢、凋零玫瑰、粉雪凍傷、方塊內窒息對玩家** | D | 0 | `hazards.rs:411` | wp44-player |
| 效果 tick、屬性、堆疊、食物與飲料效果、生物身上的效果 | A | 131＋40＋30 | `effect_parity.rs`、`mob_parity.rs` | 玩家端 weaving／oozing／wind_charged／infested 為 D |
| 附魔 43 種 | A 27／C 12／D 4 | – | `enchant_parity.rs` | D：density、wind_burst（隨重錘）、frost_walker 不結冰、soul_speed 無效果；C：channeling、flame、infinity、loyalty、lure、mending、multishot、piercing、power、punch、quick_charge、vanishing_curse |
| 吃喝、弓、弩 | A／A\* | 40／400／300 | `consume`、`item_parity.rs` | 拉弓力道、傷害為 C |
| 工具挖掘速度 | A | 280 helper＋131 | `enchant_parity.rs`、`effect_parity.rs` | 挖掘計時 C |
| **挖礦經驗（煤、青金石、鑽石、紅石、綠寶石、石英、刷怪磚…）** | D | 0 | `blocks.rs:972-985` 只掉 loot | wp44-interact |
| 桶、鍋釜、骨粉、打火石、火焰彈 | A\*／C | 桶 2,000 射線 | `item_parity.rs`、`tools.rs` | **樹苗骨粉只進一階，樹不會長**（`tools.rs:333`）；骨粉無向量；火焰彈只能點燃營火蠟燭 |
| **右鍵穿裝備（盔甲、鞘翅）** | D | 0 | – | wp44-interact |
| 剪刀、釣竿、皮帶、煙火、末影珍珠 | A\* | 528／96／18／16／20 | 各 vector | 浮標咬鉤時序、珍珠傳送傷害為 C |
| 地圖、命名牌、刷子、玩家 wind charge、**末影之眼**、發射器多數行為 | D | 0 | – | 地圖與製圖台不可用；探索地圖停在空地圖；末影之眼與終界傳送門框架缺，只能用指令進終界 |
| 移動檢查 | C | 0 | `movement.rs` | 「moved wrongly」缺：`player::server_move` 有 120 個原版向量但模擬沒接上（`region.rs:855`） |
| 飢餓、飽和、自然回血 | A\* | 隨效果向量 | `health.rs` | 自然回血無專屬向量 |
| 經驗與等級、死亡重生、睡眠、出生點 | C／A\* | – | `xp.rs`、`sleep.rs`、`weather_parity.rs` | 等級公式無向量 |
| 進度（1,866 個）、統計、配方書解鎖 | B | 0 | `advancement_check.py`、`advancements_view.py` | 54 種 trigger 中約 9 種不會觸發 |
| 姿勢（游泳、爬行）、衝刺 | C | 0 | `players.rs` | 游泳與爬行姿勢未追蹤 |

### 3.4 生物（90 種 Mob 子類，實作 83 種）

A 的證據是 `tools/MobVectors.java`（逐 tick 比對位置、速度、旋轉、health、目標、運行中的 goal／brain）：`work/m6-mobs2/vectors.jsonl` 867 個 scenario、400,824 個狀態相同；加 `finalize_parity`（自然生成的 `finalizeSpawn`，easy 27,800 筆、hard 4,100 筆）。

| 家族 | 類型 | 級別 | 向量數（主角 scenario） | 缺口 |
|---|---|---|---|---|
| 被動動物 | `pig` `cow` `sheep` `chicken` `rabbit` `fox` `wolf` `cat` `ocelot` `panda` `polar_bear` `turtle` `mooshroom` `armadillo` `sniffer` `axolotl` `goat` `frog` `tadpole` `parrot` `bat` | A（多數另有 B） | 3–29 | 豬鞍騎乘缺；狼鎧甲缺；貓晨間送禮不發生；骷髏不躲狼；`FoxStrollThroughVillage` 不執行 |
| 載具與坐騎 | `horse` `donkey` `mule` `llama` `trader_llama` `skeleton_horse` `zombie_horse` `camel` `camel_husk` `nautilus` `zombie_nautilus` | A | 2–25 | 玩家操控坐騎只有 C；背包畫面未建模 |
| 敵對（主世界） | `zombie` `husk` `drowned` `zombie_villager` `skeleton` `stray` `bogged` `parched` `creeper` `spider` `silverfish` `slime` `enderman` `endermite` `witch` `phantom` `creaking` `warden` `breeze` `guardian` `elder_guardian` `vex` | A | 2–63（不少僅 2–4 個） | **殭屍破門缺（hard 難度）**；`MoveThroughVillage` 不啟動 |
| 襲擊者 | `pillager` `vindicator` `evoker` `illusioner` `ravager` | A | 4–9＋27 個波次 | 襲擊裝備附魔擲了沒套用、破門缺、鐘不由玩家敲 |
| 下界 | `zombified_piglin` `piglin` `piglin_brute` `hoglin` `zoglin` `blaze` `ghast` `magma_cube` `wither_skeleton` `strider` | A | 3–27 | 要塞生怪覆寫缺，烈焰人與凋零骷髏在自然世界遇不到；strider 無自然生成規則 |
| 終界與頭目 | `ender_dragon` `shulker` `wither` | A | 12／7／6 | 無世界邊界 |
| 水生 | `squid` `glow_squid` `cod` `salmon` `tropical_fish` `pufferfish` | A | 3–6 | 神殿生怪覆寫缺 |
| 村民與 NPC | `villager` `wandering_trader` `iron_golem` `snow_golem` `allay` | A | 3–61 | 交易表擲法與原版不同；鐵傀儡不回村、不保衛村；堆肥不模擬 |
| **未實作（7 種）** | **`bee` `cave_spider` `dolphin` `happy_ghast` `copper_golem` `giant` `sulfur_cube`** | **D** | 0 | 存檔內保留原樣但不 tick、不送 client、`/summon` 失敗 |

能力面向：

| 能力 | 級別 | 說明 |
|---|---|---|
| AI／goal／brain | A | 83 類型全有向量；`diverges` 標記僅 2 個（`cure_zombie_villager_finish` 可能已過期、`nether_piglin_barter` 因 loot 抽籤不同） |
| 自然生成規則（上限、範圍、洗牌、放置規則） | C | `tests/mobs.rs`；分區後用每 chunk 隨機是設計的 I 類偏差；33 個類型有放置規則，strider 無放置規則 |
| 生成後初始化（裝備、騎乘者、附魔） | A | `finalize_parity`：13＋10 種 |
| **刷怪磚、試煉刷怪磚、結構生怪覆寫** | **D** | 地牢、礦坑、要塞、堡壘、沼澤小屋巫婆與貓、海底神殿守衛者、哨站掠奪者都不生怪；試煉空間與古城的「空覆寫」未實作 |
| **世界產生時的初始動物** | **D** | `pipeline.rs:11`（新區塊沒有初始動物） |
| 掉落 | A（表）／C（流程） | loot 114 張實體表 A；死亡流程、looting、熟食、XP 為 C |
| 繁殖、馴服 | A | 繁殖向量涵蓋 18 種；馴服 4 種 |
| 轉換 | A／C | 殭屍→溺屍、屍殼→殭屍、骷髏→流浪者、村民→殭屍村民、疣豬→僵屍疣豬、蝌蚪→青蛙為 A；豬布林→殭屍豬布林、雷擊轉換為 C |
| 騎乘 | A（生物騎生物）／C（玩家操控） | |
| 特殊能力 | A（多數） | 箭落點、藥水落點、shulker 彈道不比對 |

### 3.5 其他實體

| 項目 | 級別 | A 數量 | 缺口 |
|---|---|---|---|
| 物品實體物理、經驗球、點燃 TNT、掉落方塊、箭、雪球、珍珠、閃電以外的投射物 | A | 501／49／60／101／60／60 | 磁吸、撿起延遲、爆炸對生物的傷害為 C |
| 船（含箱子船）、礦車（含貨運）、煙火 | A（commit 記 24＋22／72＋84／16） | 存檔缺（`work/wp4-entities` 過期） | 氣泡柱不作用於船 |
| 閃電、經驗瓶、玩家被噴到的藥水、area effect cloud（龍息） | C／A\* | – | – |
| 畫、展示框、盔甲座、display、interaction、marker、mannequin | **D** | 0 | 存檔以 NBT 保留但不模擬、client 看不到、`/summon` 失敗 |
| 實體存檔（entities/*.mca） | B | – | `entity_persist_check.py`（原版載入 Kiln 存檔 64/64、35/35） |

### 3.6 世界生成、世界狀態、維度、規則

| 項目 | 級別 | A 數量 | 依據 | 缺口 |
|---|---|---|---|---|
| 地形密度函數與 noise router（overworld） | A | 5 seed×約 5.1M 角點×3 模式 | `tests/parity.rs` | – |
| 生物群系（overworld、nether、end） | A | 3 維度×5 seed×2,560 chunk | `tests/chunks.rs` | 只有 multi_noise 與 end source |
| 地形填充、表面規則、洞穴與峽谷、beardifier | A | 三維度各 12,800 chunk、0 不符 | `tests/chunks.rs`、`features.rs` | – |
| 特徵（29 型，樹、礦、湖、geode、冰、化石、植被等） | A | overworld 2,920,065 次放置；nether 186,873；end 14,121 | `tests/features.rs` | nether／end 只留 commit 訊息，沒有 log |
| 結構（16 種 StructureType、52 個 JSON） | A | overworld 1,234 個 start；nether／end 另計 | `features.rs --structures` | 8 個 abandoned_camp 變體未被取樣；結構內實體與方塊實體 NBT（箱子戰利品種子、spawner）無向量 |
| 出生點、基礎高度 | A | 6 seed／約 4,000 欄 | `tests/spawn.rs`、`heights.rs` | seed 12345 差 1 格（MC-55596） |
| 生成期光照、dimension types | C／B | – | – | 光照無原版比對 |
| 世界初始動物、bonus chest、blending、舊世界升級 | D | 0 | – | – |
| 天氣週期、天空暗度、降水結冰降雪、睡眠起床 | A | 515 tick／112／8 區域／12 | `weather_parity.rs` | 隨機源是 stand-in |
| 雷擊、避雷針、骷髏陷阱、`/time`、睡眠 | C | 0 | `tests/weather.rs` | – |
| 襲擊波次組成（27 場景、153 波）與襲擊者 | A | – | `raid.rs`、`mob_parity.rs` | 狀態機為 C；裝備附魔擲了沒套用 |
| 巡邏、流浪商人生成、幻翼生成 | C | 0 | `tests/raids.rs`、`wandering_trader.rs` | 幻翼週期固定 1,800 tick；`spawn_phantoms` 規則被忽略 |
| 村莊圍攻、貓生成 | **D** | 0 | – | 沒有實作 |
| 下界傳送門連結與建立、終界傳送門、閘門 | C＋B | 0 | `tests/dimensions.rs`、`dimensions_view.py` | PortalForcer 無向量；**終界傳送門框架與末影之眼缺** |
| 終界龍 AI | A | 12 | `m6s3-end` | 龍戰管理為 C |
| 世界邊界（指令回饋 A；傷害、緩衝為 C） | A／C | 38 行 | `command_diff.py` | – |
| 遊戲規則（59 條） | C | 4 條規則有指令回饋 A | – | **20 條從未被讀取**：`immediate_respawn`、`reduced_debug_info`、`spawn_phantoms`、`spawner_blocks_work`、`command_blocks_work`、`spread_vines`、`projectiles_can_break_blocks`、三種爆炸掉落衰減、`send_command_feedback`、`limited_crafting`、`forgive_dead_players`、`universal_anger`… |
| 難度、遊戲規則、`/op`、`/data storage` 重啟後 | D | 0 | – | 啟動永遠 Normal；`game_rules.dat` 只讀 `respawn_radius` |
| 世界 seed | D | 0 | – | 不讀 `world_gen_settings.dat`，預設超平坦，`/seed` 恆回 0（`commands.rs:262`）；載入原版世界時未探索處是虛空，要地形須設 `KILN_GENERATOR=noise` 與 `KILN_SEED` |

### 3.7 指令

- 原版 `commands.json` 94 個頂層指令，扣整合伺服器專用的 `publish`／`unpublish` 剩 92 個，**全部註冊**；指令樹語法 92 個逐節點對 `commands.json`（B）。
- `tools/command_diff.py`：雙伺服器現場對跑，**1,937 個比較單位全部相符**（`work/wp36/command_diff_full3.log`，約 58 個區段；`execute` 349、`scoreboard` 160、`data` 155、`item` 87、`test` 102…）。
- 級別：A 72 條、C 20 條（`time`、`weather`、`tp`、`kill`、`give`、`list`、`seed`、`msg`、`me`、`kick`、`op`、`deop`、`stop`、`difficulty`、`spawnpoint`、`setworldspawn`、`version`…只有 mock 單元測試）。
- wp44-end：`/locate structure` 已接上（`ChunkGenerator.findNearestMapStructure`：隨機散布環、同心環、`#tag`），`python tools/command_diff.py --structures` 在三個種子的生成世界上與原版逐行相符（393 行）；`/place structure|jigsaw|feature` 以即時區塊組成 `Region` 執行生成器的放置（`place_gen.rs`，原版以 `level.getRandom()` 放置，故用 `tools/place_diff.py` 做統計比對）。
- 缺口：選擇器 `level=`、`advancements=`、`predicate=` 永遠不符合；`execute if predicate` 未實作；`send_command_feedback` 被忽略；權限等級低者被拒未驗證（全以 console 等級跑）。

### 3.8 協定

驗證來源：`clientbound.txt` 162 筆（原版解碼後重編相同）、`serverbound.txt` 73 筆、26 筆實體封包、31,000 個點擊序列、10,745 個物品堆、6 筆登入。

| 狀態／方向 | 封包數 | A | B | C | D |
|---|---|---|---|---|---|
| handshake／status／login（兩向） | 1／4／11 | 0／2／5 | 1／0／5 | 0／2／1 | 0 |
| configuration（兩向） | 31 | 18 | 9 | 1 | 3 |
| play clientbound | 144 | 72（另 6 種語意 A） | 40（B／C 混合，靠真 client 或 sim） | – | 26 |
| play serverbound | 69 | 46 | 17 | – | 6 |

缺口：`open_sign_editor` 從未送出、`sign_update`／`edit_book`／中鍵挑選被丟棄；`map_item_data`、`player_chat`（簽章）、`explode` 封包未實作；`registry_data` 只帶條目名；沒有 RCON／Query／favicon。

### 3.9 持久化

- `persist_check.py`：59/59 通過；`native_roundtrip_check.py`：逐 chunk 位元組相同 2,587＋2,636 筆；`advancement_check.py` 約 27 項；物品 NBT 10,738 筆。
- A（11）：Region `.mca`、section 與 palette、方塊實體、entities、玩家核心資料、物品 NBT、advancement、level.dat 核心欄位、Anvil↔native 轉換。
- C（15）：biomes、光照、heightmap、排程 tick、POI、scoreboard、boss bar、weather、world_border、raids、dragon fight、random_sequences（Kiln 只與自己比，原版沒載入過這些檔）。
- D（11）：非 full chunk（一律丟棄重生，參考世界 2,500 個 chunk 中 1,716 個不被使用）、`InhabitedTime` 恆為 0、`wandering_trader.dat` 路徑與原版不同、`LastDeathLocation`、command storage、地圖、`ops.json`、`game_rules.dat`（只讀一項）、`difficulty`、`world_gen_settings.dat`、自訂維度；不模擬的實體只存在磁碟。

### 3.10 近似開關（只列原版可見的）

2026-10-04 使用者決定：近似開關預設全關。現有環境變數：`KILN_ENTITY_TICKING=islands|tiles`、`KILN_SCHEDULE=independent`、`KILN_LOCATOR_INTERVAL`（皆預設關）；`KILN_REGIONS`（預設分割 region，帶 I 類偏差：每 chunk 的隨機 tick 與生怪 RNG、region 級 level random、sculk 監聽順序）。設計文件 §10.3 的其他項目（AI-02…SAVE-01）在原始碼中不存在。預設就開著、不是開關的偏差：主迴圈落後時不補 tick；光照晚至多一 tick。

## 4. 缺口排序

排序依據：對生存玩家的可見度（每個伺服器都會遇到、不用特殊裝置）× 風險（偏差會破壞農場或存檔）。

| 優先 | 缺口 | 現況 | 升到 A 最便宜的路 | wp44 |
|---|---|---|---|---|
| 1 | 方塊隨機 tick：作物、農田、草蔓延、藤蔓、昆布、竹、仙人掌、甘蔗、冰雪融化、銅氧化、海龜蛋、紅石礦… | D（101 個方塊類別沒有行為） | `BlockTickVectors.java`：原版伺服器對區域呼叫 `randomTick` 與排程 tick，逐輪比對方塊、待處理 tick、亂數 | 做了（4 條分支） |
| 2 | 樹苗長成樹、骨粉長樹與草上的花 | D | 同上 harness 加 kiln-worldgen 的樹特徵；重放需在 kiln-sim 層 | 未做（計畫見下） |
| 3 | 刷怪磚與結構生怪覆寫（要塞烈焰人、地牢、沼澤小屋、神殿、哨站） | D | `MobVectors` 的 spawner scenario；`NaturalSpawner.mobsAt` 取樣向量 | 做了 |
| 4 | 終界入口：末影之眼、終界傳送門框架、`/locate structure` | D → A／B | 方塊更新向量＋`command_diff.py` 加 locate | 做了（wp44-end：`/locate structure` 393／393 與原版相符；末影之眼飛行 40 個向量逐位元相符；框架與傳送門 28 個向量相符） |
| 5 | 告示牌與書編輯、挖礦經驗、右鍵穿裝備 | D | `LootVectors` 加經驗；其餘 B（`persist_check`） | 做了 |
| 6 | 摔落傷害與落地方塊、玩家環境傷害（仙人掌、甜莓、粉雪、窒息） | C／D | `EffectVectors` 加 fall／hazard scenario | 做了 |
| 7 | 玩家打生物、橫掃、重錘 | C／D | `CombatVectors` 目標改成生物 | 做了 |
| 8 | 世界初始動物、蜜蜂與海豚等 7 種缺失生物、畫與展示框與盔甲座 | D | `MobVectors` 照樣板各加 4–8 個 scenario；存檔用 `entity_persist_check` | 未做 |
| 9 | 難度、遊戲規則、seed 與 `/op` 持久化；20 條遊戲規則沒接線 | D | `persist_check.py` 讓原版先存 Kiln 載入 | 未做 |
| 10 | 選單點擊向量補鐵砧、砂輪、附魔台、織布機、製圖台、釀造台、信標、商人 | C | `InventoryVectors.java` 選單種類清單擴充 | 未做 |
| 11 | 發射器全部行為、營火烹飪、蜂巢、鐘、講台、合成器、裝飾陶罐 | D | `ContainerVectors` 場景 | 未做 |
| 12 | 地圖與製圖台、探索地圖 | D | 需先實作 `MapItemSavedData` | 未做 |
| 13 | 「moved wrongly」、村莊圍攻、貓生成、`/place` 與選擇器 `level=` | D | 現有向量已存在（`server_move` 120 個）只需接線 | 未做 |
| 14 | 讓預設 CI 真的比對（設 `KILN_WORK`、`KILN_PARITY=1`），重錄過期向量 | 流程 | 零成本 | 工具已備（`tools/parity_suites.py`） |

## 5. 已知的過期註解（誤導讀者）

`kiln-sim/src/health.rs:12`（盾牌已實作）、`digging.rs:7`（haste 已實作）、`region.rs:1078`、`kiln-entity/src/ext_entity/trident.rs:6-7`（loyalty 已實作）、`mob/kinds/horse.rs:8`（騾已實作）、`piglin.rs:9`（長矛已實作）、`raider.rs:948`（creaking 已存在）、`kiln-sim/src/entities.rs:2889`（壓力板已實作）、`container/hopper.rs:47`（礦車已實作）、`kiln-worldgen/src/lib.rs:3` 與 `pipeline.rs:6`（結構已生成）、`kiln-proto/protocol.toml`（約 22 個 deferred 其實已實作）。

## 6. wp44 做了什麼

{{WP44}}

## 7. 重跑

```sh
# 全部 parity 套件（需 KILN_WORK 指向 work 目錄；KILN_DATAPACK 預設 <work>/generated）
python tools/parity_suites.py [--only mob_parity,fire,...]
# 方塊 hook 盤點：哪些 vanilla 方塊類別有伺服器端行為、Kiln 的原始碼有沒有參照
python tools/parity_audit.py
# 方塊自驅行為（隨機 tick、排程 tick）對原版
python tools/block_vectors.py --filter <regex>
python tools/blocks_diff.py --port 25583        # 45 個 scenario
```
