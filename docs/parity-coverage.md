# Kiln 與原版 26.3 的一致性涵蓋矩陣（wp44 稽核，wp45 整合後的狀態，wp49、wp50、wp52、wp53 補 D 項後更新）

基準：`main` 的 e5dd225 加上 wp41、wp44 及其十條子分支（wp45 整合成 `wp45-integrate`），再加 wp49（`wp49-d-gaps`，接在 wp48 之後）與 wp50（`wp50-last-d`，把第 4 節剩下的 D 項做完，見 6.4）。第 1～5 節是整合後的狀態（每個區域的 A/B/C/D 在 wp45 時重算，數字是 wp45 在 VM 上重錄並重放的結果；wp49 補完的 D 項已在各表的對應列改成現況，第 1 節的總計欄沒有重算），第 6 節記錄 wp44／wp45／wp49／wp50 做了什麼、完成了什麼、放棄了什麼。
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
| 方塊（有行為的 vanilla 方塊類別，`AuditBlockBehaviour.java` 盤點） | 218 類 | 136 | – | 2 | 58 | **22** | 隨機 tick 與排程 tick 已有向量（第 3.1 節）；剩下的 D 是只存 NBT 的方塊實體、指令方塊、氣泡柱等 |
| 方塊實體、容器、選單 | 48 | 18（本欄是 wp45 的數字；wp49 把營火、蜂巢、鐘、講台、陶罐、書架、合成器、製圖台、試煉刷怪磚、寶庫、指令方塊、日光感測器、發射器、蛋糕升為 A，逐項見 3.1、3.2 與 6.3，未重算本列） | – | 0 | 16 | 14 | 告示牌、書、刷怪磚、中鍵挑選已是 A；wp49 之後 D 剩結構／拼圖方塊 |
| 合成與配方 | 14 | 10 | – | 1 | 3 | 0 | |
| 物品欄點擊與同步 | 16 | 12 | – | 0 | 1 | 3 | |
| 戰利品 | 17 | 14 | – | 0 | 2 | 1 | |
| datapack、function、tag、predicate | 15 | 0 | – | 9 | 5 | 1 | |
| 物品元件 | 9 | 6 | – | 0 | 2 | 1 | |
| 生物（26.3 的 Mob 子類 90 種） | 90 | **90**（wp49 補上蜜蜂、海豚、快樂恐懼魔、銅傀儡、巨人、硫磺方塊，各有原版向量；其中 57 另有真 client 渲染檢查） | – | – | 0 | **0 種未實作** | 洞穴蜘蛛已實作；另有約 20 類已實作但有功能缺口（豬鞍等，見 6.3） |
| 非生物實體 | 32 | 14 | 8 | 1 | 7 | 2 | 末影之眼實體為 A |
| 戰鬥與傷害 | 25 | 18 | 1 | 0 | 5 | 1 | 玩家打生物、橫掃、重錘、摔落、環境傷害都有向量 |
| 效果與藥水 | 8 | 5 | 0 | 0 | 2 | 1 | |
| 附魔（43 種） | 43 | 27 | 0 | 0 | 12 | 4 | |
| 物品與使用 | 30 | 11 | 8 | 0 | 6 | 5 | 穿裝備、挖礦經驗、末影之眼、書與告示牌、骨粉長樹已是 A |
| 玩家系統 | 16 | 4 | 3 | 5 | 3 | 1 | 摔落、窒息、粉雪凍傷有向量 |
| 世界生成 | 30 | 20 | – | 2 | 3 | 5 | 地形與特徵是專案最紮實的部分 |
| 天氣、襲擊、維度、規則 | 32 | 8 | – | 4 | 18 | 2 | 遊戲規則 59 條中 54 條有人讀；終界入口（眼、框架）為 A；規則與難度的持久化為 B |
| 指令（92 條） | 92 | 72 | – | 0 | 20 | 0 | 缺口在子功能 |
| 協定（封包） | 213 play + 47 其他 | 121 + 25 | – | 57（B／C 混合）+ 15 | 4 | 29 + 3 | `open_sign_editor`、`sign_update`、`edit_book`、中鍵挑選已接上 |
| 持久化 | 37 | 11 | – | 6 | 15 | 5 | seed、難度、規則、ops、command storage 與原版互載（`admin_check.py` 27 項） |

讀法：A 欄多的區域（世界生成、方塊自驅行為、容器點擊、loot、指令、生物 AI、戰鬥）可以信賴；C、D 欄多的區域（只存 NBT 的方塊實體、地圖、部分選單、村莊與發射器）就是缺口。

## 2. 現有 parity 套件與本次實際執行結果

環境：`KILN_WORK` 指向有 vanilla 資料與錄製向量的 work 目錄，`KILN_DATAPACK` 指向 `work/generated`，release 建置。數字是 cargo 與測試自己印的。

| 套件（指令） | 比對對象 | 結果（wp45-integrate，VM，release） |
|---|---|---|
| `cargo test --workspace --release`，不設環境變數 | 全部單元與整合測試（parity 測試靜默略過） | 1,120 通過、0 失敗、11 ignored（144 個測試行程；main 為 1,068） |
| 同上，設 `KILN_WORK`＋`KILN_DATAPACK` | 同上，有 datapack 的測試實際執行 | 1,120 通過、0 失敗、11 ignored |
| `mob_parity`（`work/m6-mobs2/vectors.jsonl`，wp45 併入 wp41 與 wp44 的向量） | 生物逐 tick 的位置、速度、旋轉、health、目標、運行中的 goal | **988/988 scenario、457,518 個生物狀態相同**（原 867／400,824；併入的 121 個：推船與礦車、躲貓狼犰狳、蜘蛛獵鐵傀儡、刷怪磚、洞穴蜘蛛） |
| `mob_parity`（`wp34/mob_spear`、`wp36/mob_kills`、`wp36/mob_ench`） | 矛、殭屍殺村民、附魔矛 | 17／22／16 scenario 全過（8,800／3,972／4,800 狀態相同） |
| `finalize_parity`（`wp33/finalize.jsonl`、`wp34/finalize_hard.jsonl`） | 自然生成的 `finalizeSpawn`（裝備、騎乘者、附魔、level random 之後） | 31,900 筆相同、0 不同 |
| `entity_parity`（`wp45/entity/vectors.jsonl`；舊檔 `wp4-entities` 951） | 物品、經驗球、TNT、掉落方塊、投射物、載具、末影之眼等實體物理 | **1,241/1,241**（含末影之眼飛行 40 個） |
| `fire_parity`（`wp-fire`） | 火的蔓延、燃燒、老化 | 7 scenario、270/270 輪 |
| `combat`（`wp34/combat/*`） | 戰鬥、riptide、馬物品欄、矛、附魔 helper | 戰鬥 114、riptide 110、矛 127、馬物品欄 240、附魔 helper 1,381 筆（damage 648、destroy_speed 280、durability 300、protection 45、armor_effectiveness 24、modifiers 72、knockback 12），0 失敗 |
| `melee_parity`（`wp45/combat/melee.jsonl`） | 玩家對生物與玩家的近戰：橫掃、暴擊、重錘、附魔、護甲、效果、騎乘 | **690/690** |
| `spear_parity`（`wp45/combat/spear.jsonl`） | 矛的刺與衝鋒、打偏火球與風彈、刺船與礦車 | **148/148**（原 127） |
| `effect_parity`（`wp9-effects`） | 效果、飢餓、溺水、火、飲食 | 131 scenario（5,908 tick）全過 |
| `effect_parity`（`wp45/effects/vectors.jsonl`） | 上列加玩家的摔落（361）與環境傷害（110）：仙人掌、甜莓、凋零玫瑰、粉雪凍傷、窒息 | **663 全部相符（wp50 之後，沒有已知差異）**，40,133 tick |
| `container_parity`（`wp15-containers`、`wp36/containers`） | 漏斗、熔爐、比較器、唱片機 | 30 scenario（8,144 值）、14 scenario（1,400 值）全過 |
| `sculk_parity`（`m6s3-warden`） | sculk 感測器、催化劑、尖叫者 | 12 scenario、4,160/4,160 值 |
| `weather_parity`（`wx-weather`） | 天氣週期、天空暗度、降水、睡眠 | 674/674 |
| `item_parity`（`wp-itemuse`） | 視線射線（桶）、射擊、弩 | clip 1,983/2,000（互動形狀的面不同）、shoot 400/400、crossbow 69/300 逐位元（300 皆在 1e-6 內：JOML 以 float 旋轉） |
| `click_parity` 等（`KILN_PARITY=1`） | 物品欄點擊、合成、單一配方查詢、選單同步 | 31,000 序列、743,196 步、0 失敗（kiln-inventory 43 個測試） |
| `kiln-item`（`wp2-items/corpus.jsonl`） | 122 種物品元件 wire／NBT／hash／patch | 10,745 個物品堆、10,670 個元件值、整堆 64,444 通過（31 個測試） |
| `kiln-loot`（`KILN_PARITY=1`） | 戰利品表與挖礦經驗 | 36,336 case（1,445 張表）全過；挖礦經驗 2,491 個 case（624 個有經驗）全過 |
| `kiln-worldgen`（`KILN_PARITY=1`，約 18 分鐘） | 密度函數、biome、地形、表面、洞穴、特徵、結構、高度、出生點 | 34 個測試全過；每個 seed 2,560 chunk、四層 0 不符；特徵 0 不符、0 略過 |
| `kiln-storage` | Anvil 往返、原生格式、原版世界讀取 | 39 個測試全過 |
| `kiln-proto` | 封包 golden、serverbound 向量 | 60 個測試全過 |
| `kiln-command` | 指令註冊與指令樹 | 119 個測試全過；92 條指令（2,470 個節點）與 `commands.json` 相符 |
| `region_stacks`、`determinism` | region 合併與分割、決定性（含多 worker） | 5＋6 個測試全過（`determinism.rs` 的 2＋1＋6 個全過） |
| `tools/blocks_diff.py`（原版伺服器現場） | 45 個方塊 scenario 的快照 | **45/45 scenario、1,125/1,125 快照相同**（wp44 的紀錄，wp45 未重跑） |
| `block_parity`（`wp45/block/block_vectors.jsonl`，kiln-blocks） | 方塊自驅行為：隨機 tick、排程 tick、亮度與亂數，逐輪 | **185/185 scenario、26,767/26,767 步** |
| `tree_parity`（同檔，kiln-sim） | 樹苗、苗木、骨粉長樹與花草（真正的 worldgen 特徵） | **129/129 scenario、12,483/12,675 步**（其餘在原版掉落方塊之後的亂數無法重播處截止） |
| `interact_parity`（`wp45/interact/vectors.jsonl`） | 告示牌、書、右鍵穿裝備、中鍵挑選；挑選表 | **409/409 scenario；挑選表 35,723 個方塊狀態 0 不同** |
| `structure_spawns`（`wp45/spawn/structure_spawns.jsonl`） | 結構內外 `NaturalSpawner.mobsAt` 的清單 | 23,073 個樣本、184,584 張清單相同 |
| `tools/admin_check.py` | 原版與 Kiln 互載存檔：seed、規則、command storage、ops、難度 | **27/27** |
| `tools/command_diff.py`（wp45 重跑） | 指令回饋逐行 | **1,956/1,956**；`--structures` 的 `/locate structure` 393/393 |

重要觀察：

- **預設 `cargo test` 在沒有 `work/` 時靜默略過所有 parity 測試**（測試直接 return，算「通過」）。本表的數字是設了環境變數才有的。CI 若要有意義，必須設 `KILN_WORK` 與 `KILN_PARITY=1`；沒設 `KILN_PARITY=1` 時 click 序列只跑前 300 個（共 31,000）、loot 每種只跑前 400 個 case。
- **`work/` 內有些錄製檔已過期**：`work/wp4-entities/vectors.jsonl` 只有 951 個 scenario（wp45 重錄到 `work/wp45/entity/`：1,241 個）；`work/m6-combat/vectors.jsonl` 49 個，`work/wp36/combat/vectors.jsonl` 114 個。已存在的 harness 重錄後與存檔逐位元相同（例：`FireVectors` 重錄 `cmp` 一致），所以過期只表示「測試沒打到新場景」。wp45 的新向量都在 `work/wp45/`（`block`、`interact`、`combat`、`effects`、`entity`、`spawn`），`tools/parity_suites.py` 指向它們。
- **重放端的玩家不是原版的 mock 玩家**：原版 harness 的玩家不被 tick（站著、浮空、卡在方塊裡都不動），Kiln 的玩家有自己的身體（重力、阻力、落地）。重放因此要為每個 scenario 重設身體的速度、給玩家腳下一塊地板（矛），並在需要時把 `on_ground` 設成向量當時的值（riptide）；這些都寫在各 `*_parity.rs` 的註解裡。
- 所有 `tools/*Vectors.java` 現在讀 `KILN_HARNESS_PORT`，未設時自己找 25581–25583 中第一個空的埠。

## 3. 逐區域矩陣

### 3.1 方塊與方塊更新

驗證證據：

- `tools/blocks_diff.py`：原版伺服器凍結 tick 後逐 tick 步進，儲存區塊快照，Kiln 在 `TestLevel` 重播命令並逐方塊、逐排程 tick 比對。**45 個 scenario、1,125 個快照全部相同**（本次執行）。涵蓋連接形狀、彈出、水、岩漿、含水、紅石全套、活塞全套、鐵軌、樹葉距離。
- `FireVectors`（火，7 個 scenario，270 輪）、`SculkVectors`（12）、`ContainerVectors`（容器 44）、`WeatherVectors`（降水）。
- 新增：`BlockTickVectors.java`（wp44，隨機 tick 與排程 tick）：原版伺服器對一個 32×32 的區域逐輪呼叫 `randomTick` 與排程 tick，記錄方塊、待處理 tick、亮度與亂數；Kiln 在 `TestLevel` 重播並逐輪比對。wp45 重錄後共 314 個 scenario：`farm` 33、`growth` 38、`spread` 19、`ice` 8、`snow` 5、`wet` 7、`misc` 44、`end` 28、`harness` 3（這 185 個由 kiln-blocks `block_parity` 重播），另 `trees` 129（kiln-sim `tree_parity` 重播，樹由真正的 worldgen 特徵長出）。

分類以 `tools/AuditBlockBehaviour.java`（反射掃原版方塊登錄，列出每個方塊類別覆寫的 hook）加上 Kiln 原始碼的參照為準，共 218 個「有伺服器端行為」的方塊類別。wp44／wp45 之前隨機 tick 只有樹葉與岩漿；現在 `kiln_blocks::behaviour::random_tick` 分派到作物、生長、蔓延、融化、銅、海龜蛋、滴水石、樹等模組，`tools/parity_audit.py` 掃出的「完全沒提到」的類別只剩 10 個（`HangingMossBlock`、`MossyCarpetBlock`、`ShelfMushroomBlock`、`HugeMushroomBlock` 的骨粉，以及沒有行為的 `EndRodBlock`、`GlazedTerracottaBlock` 等）。

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
| 發射器、投擲器 | A／投擲器 C | 2 | `container_parity` 的 `dispenser_` 95 個 scenario＋`wind_` 28 個（wp49，見 6.3）；比較器輸出 A | 發射器：骨粉、打火石、蜂蜜瓶與玻璃瓶、發光石、蜂巢與剪刀、TNT、潛影盒、船、礦車、盔甲座、全部拋射物（箭、藥水箭、煙火、火焰彈、風彈、雪球…）、水／岩漿／粉雪／生物桶、生怪蛋、南瓜與凋零頭顱的傀儡建造、穿裝備（盔甲座、拾取戰利品的生物、馬鞍與馬鎧、熾足獸鞍）、箱子上驢與羊駝、硫磺方塊吞物、剪雪人／哞菇都有向量；玩家穿裝備、剪羊、駱駝鞍、快樂恐懼魔馬具只有 `tests/items.rs` 的單元測試或無測試；缺：刷子對犰狳、剪斷拴繩、豬鞍（豬沒有鞍欄位）、鸚鵡螺鞍與護甲 |
| 信標、附魔台、釀造台、砂輪、織布機、鐵砧選單 | C | 5 | tests/containers.rs、kiln-inventory/tests/menus.rs | 選單點擊無向量（見容器區） |
| 頭顱（凋零骷髏頭、玩家頭、豬布林頭等） | C | 7 | wither.rs（凋零建造） | 玩家頭顱 profile 放置時不套用 |
| 避雷針 | C | 1 | weather.rs 單元測試 | 氧化版為 D |
| 蜘蛛網、蓮葉 | C | 2 | mob/effects.rs、behaviour/support.rs | 玩家被蛛網減速為 D |
| 史萊姆卵（嗅探獸蛋） | C | 1 | sniffer 向量（挖掘、繁殖）；孵化為 kiln-blocks 單元 |  |
| 測試方塊（GameTest） | B | 2 | command_diff.py test 102 行；tests/gametest.rs |  |
| 農田與作物（農田、小麥、胡蘿蔔、馬鈴薯、甜菜、火炬花、瓶子草、地獄疙瘩、南瓜與西瓜莖、可可、甜莓叢、甘蔗、仙人掌、竹子與竹筍） | A | 15 | BlockTickVectors `farm` 33 個 scenario（隨機 tick、排程 tick、生長速度、農田濕度、折斷） | 骨粉對作物見 3.3 |
| 草蔓延與死亡、菌絲、地獄菌毯、樹苗（長成樹） | A | 5 | `spread` 19 個＋`trees` 129 個 scenario（樹苗、苗木、杜鵑、巨型蘑菇與草的骨粉走真正的 worldgen 特徵，`tree_parity` 129/129） | 超平坦世界沒有特徵宿主，樹苗不長（`KILN_GENERATOR=noise` 才長） |
| 藤蔓類與昆布（昆布、垂泣藤、扭曲藤、洞穴藤、藤蔓、發光地衣、紫頌花） | A | 13 | `growth` 38 個 scenario（含被剪斷後掉落的重播） |  |
| 融冰、融雪、霜冰 | A | 3 | `ice` 8、`snow` 5 個 scenario；降水形成冰與雪為 A（天氣向量） |  |
| 銅氧化與其他風化銅 | A | 14 | `misc` 的 13 個銅 scenario（`ChangeOverTimeBlock` 掃描與機率、門的下半、箱與大箱的兩半互相跟隨） |  |
| 海龜蛋、紅石礦、紫水晶、滴水石、乾燥的哈氣、硫磺 | A | 9 | `misc` 44 個 scenario（生長、掉落、滴水、鍋釜、泥） | 模擬端讓身體壓到絆線與海龜蛋 |
| 珊瑚、海綿、鷹架、絆線與鉤、目標方塊、大型垂葉 | A | 13 | `wet` 7＋`misc` 的絆線、目標、垂葉、銅燈 scenario | 鷹架塌落的實體部分為 C |
| 刷怪磚、試煉刷怪磚、寶庫 | A | 3 | 刷怪磚：mob_parity 的 `spawner_` 49 個 scenario（延遲、潛在生成物與權重、範圍、上限、光線與自訂規則、生怪蛋、`spawner_blocks_work`、亂數與存檔）；試煉刷怪磚 `interact_parity` `trial_` 14 個、寶庫 `vault_` 19 個 scenario（wp49，見 6.3） | 試煉刷怪磚的怪物位置由 level 亂數決定，向量用給定位置避開；不祥試煉的物品雨由 wp50 補上（`ominous_item_spawner`：`ominous50` 15 個 `container_parity` scenario＋`tests/ominous_trial.rs`） |
| 告示牌、懸掛式告示牌 | A | 4 | `interact_parity` 的 sign／equip／book／pick 共 409 個 scenario（編輯與編輯鎖、染色、上蠟、螢光墨囊、點擊事件、牆上與懸掛式放置） | 懸掛告示牌的存活（支撐消失會掉）未模擬 |
| 營火、蜂巢、鐘、講台、裝飾陶罐、書架（雕紋）、合成器、製圖台 | A | 8 | wp49：`interact_parity` 的營火＋陶罐＋書架 `b49` 67、鐘 76、蜂巢 22、講台 22、合成器 99、陶罐 44、製圖台與地圖 21；`container_parity` 的營火 6、蜂巢 6、講台 10、陶罐 14、合成器 26（見 6.3） | 原清單中的擱板（shelf）與潛影導管沒有獨立向量（未驗證，沿用 C） |
| 指令方塊（普通、連鎖、重複；條件式）、結構方塊、拼圖方塊 | A（指令方塊、結構方塊、拼圖方塊） | 3 | wp49：`interact_parity` 的指令方塊 46 個＋`container_parity` 的 `cmdblock` 24 個 scenario（執行、條件、連鎖、重複、紅石、自動、`command_block_output`、礦車指令方塊） | wp50 補上結構方塊與拼圖方塊（`interact_parity` 的 `structure50` 88 個 scenario，見 6.4）；缺口：存檔時不收集範圍內的實體、拼圖方塊 `generate` 的 `keepJigsaws` 旗標不作用 |
| 其他：日光感測器、旗幟、花盆、蛋糕、凋零玫瑰、終界傳送門框架、氣泡柱、可摔落的刷子方塊 | A（wp50 補上旗幟圖樣） | 9 | 終界傳送門框架：`end` 28 個 scenario；凋零玫瑰：`effect_parity` `haz_wither_rose_*`；wp49：日光感測器 `daylight` 4、蛋糕 `cake` 23、花盆（見上）、氣泡柱 `block/bubble` 6、刷子 `brush` 17 個 scenario；wp50：旗幟圖樣（織布機、盾牌、鍋釜洗旗）`banner50` 19、`cauldron50` 170 個 scenario | – |

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
| 發射器 | A | 95＋28 | `container_parity` 的 `dispenser_`／`wind_`（wp49，見 3.1 與 6.3） | wp50 補上刷子對犰狳、剪斷拴繩、豬鞍、鸚鵡螺鞍與護甲（`container50` 的 `c50` 45 個 scenario，見 6.4） |
| 織布機選單 | A | 1,500 序列／35,988 步 | wp50：`kiln-inventory/tests/click_parity.rs`（`loom_clicks`） | 見 6.4 |
| 投擲器、終界箱、信標、附魔台、鐵砧、砂輪、商人選單、羊駝物品欄、床、重生錨 | C | 0 | `tests/*`、`kiln-inventory/tests/menus.rs` | 選單點擊向量只涵蓋 14 種選單（見下） |
| 合成器、講台、製圖台、裝飾陶罐、書架、營火、鐘、蜂巢 | A | 見 3.1 | wp49 的 `interact_parity`／`container_parity` 向量（見 3.1 與 6.3） | – |
| 刷怪磚、試煉刷怪磚、寶庫 | A | 49＋14＋19 | `mob_parity` 的 `spawner_`＋`cavespider_`；`interact_parity` 的 `trial_`、`vault_` | – |
| 告示牌編輯、染色、上蠟、書與筆 | A | 194＋21 | `interact_parity.rs`（`sign_*`、`book_*`）；未放行文字過濾（聊天過濾器）與書的 `resolveBookComponents` | 橫幅與頭顱放置資料仍 D |
| 合成配方：有形、無形、轉換、特殊、冶煉、高爐、煙燻、營火烹飪、釀造、切石、鍛造 | A | 827／375／33＋全部特殊配方；冶煉類 116 配方；釀造 279 | `crafting_parity.rs`、`single_parity.rs` | 有 4 個配方沒被向量打到 |
| 配方書顯示與放置 | C | 0 | `recipe_book.rs`、`menus.rs` | 無封包位元組比對 |
| 物品欄點擊（PICKUP、QUICK_MOVE、PICKUP_ALL、QUICK_CRAFT、SWAP、THROW、CLONE、創造槽、按鈕、關閉、無效封包） | A | 約 74 萬步／31,000 序列 | `click_parity.rs` | 向量只涵蓋 13 種選單，缺鐵砧、砂輪、附魔台、織布機、製圖台、釀造台、信標、商人、講台、合成器 |
| 選單同步（state id、過期預測、竄改預測） | A | 31,000 序列 | `sync_parity.rs` | – |
| 中鍵挑選方塊（Pick Block） | A | 79 個 scenario＋35,723 個方塊狀態的挑選表 | `interact_parity.rs`、`pick_table_parity` | 創造模式帶方塊實體資料（Ctrl＋中鍵）、橫幅與裝飾陶罐的圖樣為 D；Bundle 選取、`ContainerSlotStateChanged` 仍被丟棄 |
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
| 玩家對生物近戰、護甲、附魔、效果、騎乘、水中 | A | 690 個 `melee` scenario（`melee_parity.rs`，每項逐位元：生命、擊退、聲音、粒子、耐久、經驗、封包） | `CombatVectors.java --filter melee` | 見重錘與橫掃 |
| 橫掃攻擊（對生物與玩家） | A | `melee/sweep` 60 個 | `melee_parity.rs` |  |
| 重錘（smash、density、breach、wind burst 與擊退爆炸） | A | `melee/mace` 196 個 | `melee_parity.rs` |  |
| 矛（玩家與生物） | A | 127＋17 | `spear_parity.rs` | – |
| 三叉戟 riptide | A | 110 | `combat_parity.rs` | – |
| 盾牌、不死圖騰 | C | 0 | `shield.rs`、`tests/health.rs` | – |
| 摔落傷害（safe_fall_distance、摔落乘數、乾草 0.2、床 0.5、史萊姆與蜂蜜、蜘蛛網、梯子、粉雪、水與氣泡柱）、方塊彈起 | A | `effect_parity` 的 `fall_*` 361 個 scenario（全部相符，wp45 的 7 個差異 wp49 補完） | `EffectVectors.java` | – |
| 溺水、火、岩漿、岩漿塊、營火、飢餓與餓死 | A | 11＋17＋3 | `effect_parity.rs` | – |
| 仙人掌、甜莓叢、凋零玫瑰、粉雪凍傷、方塊內窒息（含冷卻、難度、盔甲、保護、抗性） | A | `haz_*` 110 個 scenario（全部相符，wp45 的 5 個差異 wp49、wp50 補完） | `EffectVectors.java` | – |
| 效果 tick、屬性、堆疊、食物與飲料效果、生物身上的效果 | A | 131＋40＋30 | `effect_parity.rs`、`mob_parity.rs` | 玩家端 weaving／oozing／wind_charged／infested 為 D |
| 附魔 43 種 | A 31／C 12／D 0 | – | `enchant_parity.rs`、`melee_parity.rs`（density、wind_burst、breach 隨重錘）；wp49：霜行者與靈魂疾行者 `effect_parity` 的 `ench_` 17 個 scenario（`enchant_loc.rs`） | 靈魂疾行者的靴子磨損抽的是 level 亂數，與音效、腳步共用一條序列，向量不比對；C：channeling、flame、infinity、loyalty、lure、mending、multishot、piercing、power、punch、quick_charge、vanishing_curse |
| 吃喝、弓、弩 | A／A\* | 40／400／300 | `consume`、`item_parity.rs` | 拉弓力道、傷害為 C |
| 工具挖掘速度 | A | 280 helper＋131 | `enchant_parity.rs`、`effect_parity.rs` | 挖掘計時 C |
| 挖礦經驗（煤、青金石、鑽石、紅石、綠寶石、石英、刷怪磚、sculk 方塊） | A | `kiln-loot` 的 `block_experience` 向量：每個方塊與每種工具的量、經驗球個數與亂數抽取 | `vanilla_parity.rs`（`KILN_PARITY=1`） | 經驗球本身用 26.3 的建構子抽取 |
| 桶、鍋釜、骨粉、打火石、火焰彈 | A\*／C | 桶 2,000 射線；骨粉：作物（`farm`）、苗木與草（`trees` 129）逐位元 | `item_parity.rs`、`tree_parity.rs`、`tools.rs` | 火焰彈只能點燃營火蠟燭；超平坦世界沒有特徵宿主，樹苗與草的骨粉不長 |
| 右鍵穿裝備（盔甲、鞘翅；交換、創造、詛咒、冷卻、副手、冒險模式） | A | `equip_*` 115 個 scenario（`interact_parity.rs`） | `InteractVectors.java` |  |
| 剪刀、釣竿、皮帶、煙火、末影珍珠 | A\* | 528／96／18／16／20 | 各 vector | 浮標咬鉤時序、珍珠傳送傷害為 C |
| 末影之眼（放進框架、開傳送門、飛行、落下或碎裂）、`/locate` 的 `#eye_of_ender_located` | A | 40 個飛行向量（`entity_parity`）、28 個框架與環形 scenario（`block_parity` 的 `end`） | `EntityVectors.java`、`BlockTickVectors.java` |  |
| 地圖（含製圖台、旗幟、展示框、探索地圖）、命名牌、刷子、玩家 wind charge、發射器 | A | `interact_parity` 的 `maps_`／`carto_`／`mframe_` 23 個、`explore` 11 個、`brush_` 17 個；`container_parity` 的 `wind_` 28 個＋`dispenser_` 95 個（wp49，見 6.3） | `InteractVectors.java`、`ExploreMapVectors.java`、`ContainerVectors.java` | 命名牌（Name Tag）見 6.3；地圖顏色以世界地形算（同 worldgen 的 A 範圍） |
| 移動檢查 | A | 27（wp50 `moves50`） | `movement.rs`、`region.rs` 的 `handle_move`、`phantom.rs` | 「moved wrongly」已接上（伺服器身體 `player::server_move` 的終點對上用戶端宣稱的位置）；被退回的移動不計摔落傷害；卡在蜘蛛網等的第二步沒有向量（原版 harness 不 tick 玩家） |
| 飢餓、飽和、自然回血 | A\* | 隨效果向量 | `health.rs` | 自然回血無專屬向量 |
| 經驗與等級、死亡重生、睡眠、出生點 | C／A\* | – | `xp.rs`、`sleep.rs`、`weather_parity.rs` | 等級公式無向量 |
| 進度（1,866 個）、統計、配方書解鎖 | B | 0 | `advancement_check.py`、`advancements_view.py` | 54 種 trigger 中約 9 種不會觸發 |
| 姿勢（游泳、爬行、強迫蹲）、衝刺 | A\* | `effect_parity` 的 `haz_wall_ceiling_slab_top` 等（wp49 `Player.updatePlayerPose`） | `pose.rs`、`players.rs` | 衝刺本身無向量 |

`effect_parity` 663 個 scenario（wp50 之後）**全部逐項相符（40,133 tick）**，`effect_parity.rs` 的 `KNOWN_GAPS` 清單是空的（測試對這份清單雙向把關：清單外的差異會失敗，清單上的轉為相符也會失敗，要求移出清單）。wp45 的 12 個差異中，粉雪落下、氣泡柱飢餓、床彈起、只有頭卡住、低天花板強迫蹲姿共 11 個在 wp49 補了：玩家姿勢（游泳、爬行、強迫蹲）、`aiStep` 的 0.003 速度歸零、氣泡柱推玩家（含一次 level 亂數）、封包移動走伺服器身體（`player::server_move`）；最後一個 `haz_snow_lava_clears` 在 wp50 查明是錄製端的假象（見下表）：

| 曾經的差異 | 原因與處理 |
|---|---|
| `haz_snow_lava_clears` | 不是 Kiln 的缺口。harness 的「影子用戶端」是第二個站在同一格粉雪裡的玩家，岩漿出現那個 tick 它先著火、先融掉粉雪（把它的 `InsideBlockEffectType` 效果逐一印出來：`LAVA_IGNITE`、`EXTINGUISH` 發生在被測玩家被處理之前），被測玩家於是在粉雪消失後才被點燃，火不被撲滅；單一玩家（Kiln 的情形）則是點燃之後同一步被粉雪撲滅。`EffectVectors` 加 `quietShadow`：這個 scenario 錄製時影子不套用方塊效果，重錄後 Kiln 逐項相符，`KNOWN_GAPS` 清空 |

### 3.4 生物（90 種 Mob 子類，wp49 之後全部實作）

A 的證據是 `tools/MobVectors.java`（逐 tick 比對位置、速度、旋轉、health、目標、運行中的 goal／brain）：`work/m6-mobs2/vectors.jsonl` 在 wp45 併入 wp41（推船與礦車、躲貓狼犰狳、蜘蛛獵鐵傀儡）與 wp44（刷怪磚、洞穴蜘蛛）的向量後共 988 個 scenario、457,518 個狀態相同（原 867 個、400,824 個狀態）；wp49 再併入蜜蜂、海豚、快樂恐懼魔、銅傀儡、巨人、硫磺方塊的 144 個 scenario，共 **1,132 個 scenario、553,706 個狀態相同**；加 `finalize_parity`（自然生成的 `finalizeSpawn`，easy 27,800 筆、hard 4,100 筆）。

| 家族 | 類型 | 級別 | 向量數（主角 scenario） | 缺口 |
|---|---|---|---|---|
| 被動動物 | `pig` `cow` `sheep` `chicken` `rabbit` `fox` `wolf` `cat` `ocelot` `panda` `polar_bear` `turtle` `mooshroom` `armadillo` `sniffer` `axolotl` `goat` `frog` `tadpole` `parrot` `bat` | A（多數另有 B） | 3–29 | 豬鞍與胡蘿蔔釣竿騎乘在 wp50 補上（`eq50_pig*`）；狼鎧甲缺；貓晨間送禮不發生；`FoxStrollThroughVillage` 不執行。苦力怕、骷髏、蜘蛛、襲擊者會躲貓、狼、犰狳、嚎叫者（wp41，`avoid_*` 向量） |
| 載具與坐騎 | `horse` `donkey` `mule` `llama` `trader_llama` `skeleton_horse` `zombie_horse` `camel` `camel_husk` `nautilus` `zombie_nautilus` | A | 2–25 | 鸚鵡螺（含殭屍鸚鵡螺）的鞍與護甲在 wp50 補上（`eq50_nautilus*`、`eq50_zombie_nautilus*`）；玩家操控坐騎只有 C；背包畫面未建模 |
| 敵對（主世界） | `zombie` `husk` `drowned` `zombie_villager` `skeleton` `stray` `bogged` `parched` `creeper` `spider` `cave_spider` `silverfish` `slime` `enderman` `endermite` `witch` `phantom` `creaking` `warden` `breeze` `guardian` `elder_guardian` `vex` | A | 2–63（不少僅 2–4 個） | **殭屍破門缺（hard 難度）**；`MoveThroughVillage` 不啟動 |
| 襲擊者 | `pillager` `vindicator` `evoker` `illusioner` `ravager` | A | 4–9＋27 個波次 | 襲擊裝備附魔擲了沒套用、破門缺、鐘不由玩家敲 |
| 下界 | `zombified_piglin` `piglin` `piglin_brute` `hoglin` `zoglin` `blaze` `ghast` `magma_cube` `wither_skeleton` `strider` | A | 3–27 | 要塞生怪覆寫缺，烈焰人與凋零骷髏在自然世界遇不到；strider 無自然生成規則；strider 的鞍與胡蘿蔔釣竿在 wp50 補上（`eq50_strider*`） |
| 終界與頭目 | `ender_dragon` `shulker` `wither` | A | 12／7／6 | 無世界邊界 |
| 水生 | `squid` `glow_squid` `cod` `salmon` `tropical_fish` `pufferfish` | A | 3–6 | 神殿生怪覆寫缺 |
| 村民與 NPC | `villager` `wandering_trader` `iron_golem` `snow_golem` `allay` | A | 3–61 | 交易表擲法與原版不同；鐵傀儡不回村、不保衛村；堆肥不模擬 |
| wp49 補上的 6 種 | `bee`（30）`dolphin`（27）`happy_ghast`（20）`copper_golem`（34；含箱子搬運、風化與雕像、蜜蠟與斧、雷擊、鐵傀儡送花）`giant`（4）`sulfur_cube`（29；12 種原型、吞物、剪、餵食、分裂、TNT 球） | A | 144 個 scenario 併入 `m6-mobs2` | 硫磺方塊的玩家推擠、衝刺擊退、接觸傷害對生物沒有向量可比（harness 不 tick 玩家）；快樂恐懼魔騎乘與鞍具只有 C |

能力面向：

| 能力 | 級別 | 說明 |
|---|---|---|
| AI／goal／brain | A | 83 類型全有向量；`diverges` 標記僅 2 個（`cure_zombie_villager_finish` 可能已過期、`nether_piglin_barter` 因 loot 抽籤不同） |
| 自然生成規則（上限、範圍、洗牌、放置規則） | C | `tests/mobs.rs`；分區後用每 chunk 隨機是設計的 I 類偏差；33 個類型有放置規則，strider 無放置規則 |
| 生成後初始化（裝備、騎乘者、附魔） | A | `finalize_parity`：13＋10 種 |
| 刷怪磚、試煉刷怪磚、結構生怪覆寫 | A（不祥試煉的物品生成實體除外） | 刷怪磚：`spawner_`＋`cavespider_` 49 個 scenario（49,494 個狀態相同）；試煉刷怪磚 14 個 scenario（`interact_parity`）；結構覆寫：`NaturalSpawner.mobsAt` 取樣 23,073 個位置、184,584 張清單相同（要塞、堡壘、沼澤小屋、神殿、哨站、試煉空間、古城…） |
| 世界產生時的初始動物 | A\* | wp49：`InitialMobVectors.java` 7 個世界（每個 169 個區塊的窗口）；Kiln 在區塊產生時放的動物（含 `isValidSpawn` 逐種放置條件）有 161 隻原版的動物中 114 隻位置與朝向逐位元相同，其餘落在原版自己的順序相依範圍內（原版的結果取決於區塊產生的順序，測試以重跑原版量到的容許度比對，`tests/initial_mobs.rs`） |
| 掉落 | A（表）／C（流程） | loot 114 張實體表 A；死亡流程、looting、熟食、XP 為 C |
| 繁殖、馴服 | A | 繁殖向量涵蓋 18 種；馴服 4 種 |
| 轉換 | A／C | 殭屍→溺屍、屍殼→殭屍、骷髏→流浪者、村民→殭屍村民、疣豬→僵屍疣豬、蝌蚪→青蛙為 A；豬布林→殭屍豬布林、雷擊轉換為 C |
| 騎乘 | A（生物騎生物）／C（玩家操控） | |
| 特殊能力 | A（多數） | 箭落點、藥水落點、shulker 彈道不比對 |

### 3.5 其他實體

| 項目 | 級別 | A 數量 | 缺口 |
|---|---|---|---|
| 物品實體物理、經驗球、點燃 TNT、掉落方塊、箭、雪球、珍珠、閃電以外的投射物、末影之眼 | A | 1,241 個 scenario（`entity_parity`，舊檔 951；末影之眼 40 個） | 磁吸、撿起延遲、爆炸對生物的傷害為 C。玩家的矛與拳頭打偏火球與風彈（wp41，`stab_projectile`、`melee_projectile`）與生物推船與礦車（`push_*` 76 個 scenario）為 A |
| 船（含箱子船）、礦車（含貨運）、煙火 | A（commit 記 24＋22／72＋84／16） | 存檔缺（`work/wp4-entities` 過期） | 氣泡柱不作用於船 |
| 閃電、經驗瓶、玩家被噴到的藥水、area effect cloud（龍息） | C／A\* | – | – |
| 畫、展示框、盔甲座、display（方塊／物品／文字）、interaction、marker | A | wp49：`interact_parity` 的 `frames_` 55、`stand_` 142、`mframe_` 2 個 scenario；`entity_nbt` 127 個（`EntityNbtVectors.java`：原版載入並存出的 NBT 與 Kiln 逐欄相同的 122 個，5 個留給模擬端） | 文字顯示的選擇器與分數解析在模擬端做，向量只比存檔欄位 |
| 風彈（玩家丟出與發射器射出的 `wind_charge`）、被拋射物打中的載具／盔甲座／畫 | A | wp49：`container_parity` 的 `wind_` 27 個 scenario（牆、地板、各種生物、載具、離開載入範圍） | 拋射物對 `#redirectable_projectile` 與 `BlockAttachedEntity` 的撞擊判定補齊（`Entity.canBeHitByProjectile`） |
| mannequin、cushion、ominous_item_spawner | A（wp50） | cushion 152、mannequin 46＋`entity_nbt` 53、ominous 15 | `interact_parity` 的 `cushion50`／`mannequin50`；`entity_nbt.rs` 的 `mannequin.jsonl`；`container_parity` 的 `ominous50` | cushion 的流體、活塞與中鍵挑選、mannequin 的方塊音效（harness 不錄生物的方塊音效）未比對，見 6.4 |
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
| 巡邏、流浪商人生成、幻翼生成 | C | 0 | `tests/raids.rs`、`wandering_trader.rs`、`phantom.rs` | `spawn_phantoms` 規則已讀；區塊的 `InhabitedTime` 累計、存檔，並用於自然生成、幻翼與 `/summon` 的區域難度；幻翼週期仍固定 1,800 tick |
| 村莊圍攻、貓生成 | **D** | 0 | – | 沒有實作 |
| 下界傳送門連結與建立、終界傳送門與框架、閘門 | C＋B／框架 A | 0（框架見 3.3 末影之眼） | `tests/dimensions.rs`、`dimensions_view.py` | PortalForcer 無向量 |
| 終界龍 AI | A | 12 | `m6s3-end` | 龍戰管理為 C |
| 世界邊界（指令回饋 A；傷害、緩衝為 C） | A／C | 38 行 | `command_diff.py` | – |
| 遊戲規則（59 條） | C／B | 4 條規則有指令回饋 A；難度與 24 條規則的持久化 B | `admin_check.py` | 59 條中 54 條有人讀；沒人讀的 5 條：`command_block_output`、`command_blocks_work`（沒有指令方塊）、`max_command_forks`、`max_command_sequence_length`、`max_minecart_speed` |
| 難度（與鎖）、遊戲規則、`/op`（`ops.json` 含等級）、`/data storage` 重啟後 | B | 27 項互載檢查 | `tools/admin_check.py`（原版建立存檔、Kiln 載入並回答相同；Kiln 改完存檔、原版載入） | 登入封包帶 hardcore、reduced_debug_info、死亡畫面、limited_crafting 與雜湊 seed |
| 世界 seed | B | 同上 | `world_gen_settings.dat` 讀寫、`/seed`、`BiomeManager.obfuscateSeed`（登入封包的雜湊 seed 與原版相同，golden 向量由原版重新驗證） | 載入原版世界時未探索處仍是虛空，要地形須設 `KILN_GENERATOR=noise`（此時採用存檔的 seed） |

### 3.7 指令

- 原版 `commands.json` 94 個頂層指令，扣整合伺服器專用的 `publish`／`unpublish` 剩 92 個，**全部註冊**；指令樹語法 92 個逐節點對 `commands.json`（B）。
- `tools/command_diff.py`：雙伺服器現場對跑，**wp45 重跑 1,956 行全部相符**（舊紀錄 1,937 個比較單位，`work/wp36/command_diff_full3.log`，約 58 個區段；`execute` 349、`scoreboard` 160、`data` 155、`item` 87、`test` 102…）。
- 級別：A 72 條、C 20 條（`time`、`weather`、`tp`、`kill`、`give`、`list`、`seed`、`msg`、`me`、`kick`、`op`、`deop`、`stop`、`difficulty`、`spawnpoint`、`setworldspawn`、`version`…只有 mock 單元測試）。
- wp44-end：`/locate structure` 已接上（`ChunkGenerator.findNearestMapStructure`：隨機散布環、同心環、`#tag`），`python tools/command_diff.py --structures` 在三個種子（1、4242、-777777）的生成世界上與原版逐行相符（393／393 行）；`/place structure|jigsaw|feature` 以即時區塊組成 `Region` 執行生成器的放置（`place_gen.rs`，原版以 `level.getRandom()` 放置，故用 `tools/place_diff.py` 做統計比對）。
- 缺口：選擇器 `level=`、`advancements=`、`predicate=` 永遠不符合；`execute if predicate` 未實作；`send_command_feedback` 被忽略；權限等級低者被拒未驗證（全以 console 等級跑）。

### 3.8 協定

驗證來源：`clientbound.txt` 162 筆（原版解碼後重編相同）、`serverbound.txt` 73 筆、26 筆實體封包、31,000 個點擊序列、10,745 個物品堆、6 筆登入。

| 狀態／方向 | 封包數 | A | B | C | D |
|---|---|---|---|---|---|
| handshake／status／login（兩向） | 1／4／11 | 0／2／5 | 1／0／5 | 0／2／1 | 0 |
| configuration（兩向） | 31 | 18 | 9 | 1 | 3 |
| play clientbound | 144 | 72（另 6 種語意 A） | 40（B／C 混合，靠真 client 或 sim） | – | 26 |
| play serverbound | 69 | 46 | 17 | – | 6 |

缺口：`map_item_data`、`player_chat`（簽章）、`explode` 封包未實作（`open_sign_editor`、`sign_update`、`edit_book`、`pick_item_from_block`、`set_held_slot`、`open_book` 已接上並有向量）；`registry_data` 只帶條目名；沒有 RCON／Query／favicon。

### 3.9 持久化

- `persist_check.py`：59/59 通過；`native_roundtrip_check.py`：逐 chunk 位元組相同 2,587＋2,636 筆；`advancement_check.py` 約 27 項；物品 NBT 10,738 筆。
- A（11）：Region `.mca`、section 與 palette、方塊實體、entities、玩家核心資料、物品 NBT、advancement、level.dat 核心欄位、Anvil↔native 轉換。
- C（15）：biomes、光照、heightmap、排程 tick、POI、scoreboard、boss bar、weather、world_border、raids、dragon fight、random_sequences（Kiln 只與自己比，原版沒載入過這些檔）。
- B（6，wp44-admin）：`world_gen_settings.dat` 的 seed、`level.dat` 的難度與鎖、`game_rules.dat`、command storage、`ops.json`、區塊的 `InhabitedTime`（`admin_check.py` 27 項互載）。
- D（5）：非 full chunk（一律丟棄重生，參考世界 2,500 個 chunk 中 1,716 個不被使用）、`wandering_trader.dat` 路徑與原版不同、`LastDeathLocation`、地圖、自訂維度；不模擬的實體只存在磁碟。

### 3.10 近似開關（只列原版可見的）

2026-10-04 使用者決定：近似開關預設全關。現有環境變數：`KILN_ENTITY_TICKING=islands|tiles`、`KILN_SCHEDULE=independent`、`KILN_LOCATOR_INTERVAL`（皆預設關）；`KILN_REGIONS`（預設分割 region，帶 I 類偏差：每 chunk 的隨機 tick 與生怪 RNG、region 級 level random、sculk 監聽順序）。設計文件 §10.3 的其他項目（AI-02…SAVE-01）在原始碼中不存在。預設就開著、不是開關的偏差：主迴圈落後時不補 tick；光照晚至多一 tick。

## 4. 缺口排序

排序依據：對生存玩家的可見度（每個伺服器都會遇到、不用特殊裝置）× 風險（偏差會破壞農場或存檔）。

| 優先 | 缺口 | 現況（wp45 之後） | 升到 A 最便宜的路 | 結果 |
|---|---|---|---|---|
| 1 | 方塊隨機 tick：作物、農田、草蔓延、藤蔓、昆布、竹、仙人掌、甘蔗、冰雪融化、銅氧化、海龜蛋、紅石礦… | A（185 個 scenario 逐輪相符） | `BlockTickVectors.java` | 完成（farming、growth、misc 三條分支） |
| 2 | 樹苗長成樹、骨粉長樹與草上的花 | A（129 個 scenario 相符；超平坦世界沒有特徵宿主，不長） | 同 harness＋kiln-worldgen 的特徵宿主 | 完成（wp44-trees）；超平坦世界給特徵宿主是新缺口 |
| 3 | 刷怪磚與結構生怪覆寫（要塞烈焰人、地牢、沼澤小屋、神殿、哨站） | A（刷怪磚 49 個 scenario、結構覆寫 184,584 張清單） | `MobVectors`、`SpawnVectors` | 完成（wp44-spawner）；試煉刷怪磚、寶庫未做 |
| 4 | 終界入口：末影之眼、終界傳送門框架、`/locate structure` | A／B | 方塊更新向量＋`command_diff.py --structures` | 完成（wp44-end：locate 393／393，眼的飛行 40 個、框架 28 個） |
| 5 | 告示牌與書編輯、挖礦經驗、右鍵穿裝備、中鍵挑選 | A | `InteractVectors.java`、loot 的 `block_experience` | 完成（wp44-interact）；創造模式帶資料的中鍵挑選放棄 |
| 6 | 摔落傷害與落地方塊、玩家環境傷害（仙人掌、甜莓、粉雪、窒息） | A（663／663，wp50 之後） | `EffectVectors` 加 fall／hazard scenario | 完成（wp44-player）；wp45 的 12 個差異由 wp49 補 11 個、wp50 補最後 1 個（見 3.3） |
| 7 | 玩家打生物、橫掃、重錘 | A（690／690） | `CombatVectors` 目標改成生物 | 完成（wp44-combat） |
| 8 | 世界初始動物、蜜蜂與海豚等 6 種缺失生物、畫與展示框與盔甲座 | A（wp49 之後） | `MobVectors` 照樣板各加 4–8 個 scenario；存檔用 `entity_persist_check` | 完成（wp49，見 6.3）；mannequin、cushion、ominous_item_spawner 在 wp50 補上（見 6.4） |
| 9 | 難度、遊戲規則、seed 與 `/op` 持久化；遊戲規則接線 | B（27 項互載；59 條規則 54 條有人讀） | 原版先存、Kiln 載入，反過來再一次 | 完成（wp44-admin） |
| 10 | 選單點擊向量補鐵砧、砂輪、附魔台、織布機、製圖台、釀造台、信標、商人 | C（織布機已升 A） | `InventoryVectors.java` 選單種類清單擴充 | 織布機在 wp50 完成（1,500 序列）；其餘未做 |
| 11 | 發射器全部行為、營火烹飪、蜂巢、鐘、講台、合成器、裝飾陶罐 | A（wp49 之後） | `ContainerVectors` 場景 | 完成（wp49，見 6.3）；發射器的刷子對犰狳、剪斷拴繩、豬鞍等在 wp50 補上 |
| 12 | 地圖與製圖台、探索地圖 | A（wp49 之後） | 需先實作 `MapItemSavedData` | 完成（wp49：`maps_`／`carto_`／`mframe_` 與 `explore` 向量） |
| 13 | 「moved wrongly」、村莊圍攻、貓生成、選擇器 `level=` | 村莊圍攻與貓生成 C（已實作，無原版向量）／「moved wrongly」A | 現有 `server_move` 物理接線，另錄 `moves50` | 圍攻與貓生成在 wp49 寫完；「moved wrongly」在 wp50 完成（27 個 scenario） |
| 14 | 讓預設 CI 真的比對（設 `KILN_WORK`、`KILN_PARITY=1`），重錄過期向量 | 流程 | 零成本 | `tools/parity_suites.py` 已含 wp44／wp45 的套件；wp45 重錄了 block、interact、melee、spear、effects、entity、spawn 向量，並把 wp41／wp44 的生物向量併入 `m6-mobs2` |

## 5. 已知的過期註解（誤導讀者）

`kiln-sim/src/health.rs:12`（盾牌已實作）、`digging.rs:7`（haste 已實作）、`region.rs:1078`、`kiln-entity/src/ext_entity/trident.rs:6-7`（loyalty 已實作）、`mob/kinds/horse.rs:8`（騾已實作）、`piglin.rs:9`（長矛已實作）、`raider.rs:948`（creaking 已存在）、`kiln-sim/src/entities.rs:2889`（壓力板已實作）、`container/hopper.rs:47`（礦車已實作）、`kiln-worldgen/src/lib.rs:3` 與 `pipeline.rs:6`（結構已生成）、`kiln-proto/protocol.toml`（約 22 個 deferred 其實已實作）。

## 6. wp44／wp45 做了什麼

wp44 先做稽核（第 0～5 節的矩陣、`tools/parity_audit.py`、`tools/parity_suites.py`、`BlockTickVectors.java` 的框架、所有 harness 自動選 25581–25583 的埠、`/seed` 與亂數序列用世界 seed），再依缺口排序開十條子分支平行補洞。使用者的週用量上限讓其中七條停在「WIP（stopped for weekly usage limit）」的存檔提交，wp45 把它們和 wp41 一起整合成 `wp45-integrate`。

### 6.1 各分支做了什麼

| 分支 | 內容 | wp45 重放的結果 |
|---|---|---|
| wp41-small-gaps | 玩家的矛與拳頭打偏火球與風彈（`deflectProjectile`）、execute on controller、生物與船／礦車互推、船上乘客跟船轉向、苦力怕／骷髏／蜘蛛／襲擊者躲貓狼犰狳、蜘蛛獵鐵傀儡、風彈在水中的慣性 | 生物向量 76 個 scenario（8,950 個狀態）、矛 148 個 scenario 相符 |
| wp44-farming | 農田、作物、瓶子草、地獄疙瘩、莖、可可、甜莓、甘蔗、仙人掌、竹子的隨機與排程 tick | `farm` 33 個 scenario |
| wp44-misc | 銅氧化、海龜蛋、紅石礦、紫水晶、乾燥的哈氣、滴水石與硫磺尖刺、絆線、目標、大型垂葉、銅燈；模擬端讓身體壓到絆線與海龜蛋 | `misc` 44 個 scenario |
| wp44-growth | 草與菌絲蔓延、融雪融冰與霜冰、生長植物、藤蔓、蘑菇、菌毯、紫頌花、珊瑚、鷹架、海綿 | `growth` 38、`spread` 19、`ice` 8、`snow` 5、`wet` 7 個 scenario |
| wp44-trees | 樹苗、苗木、杜鵑、巨型蘑菇與草的骨粉走真正的 worldgen 特徵（kiln-blocks 的 `FeatureHost`，kiln-worldgen 的 `WorldgenHost` 把特徵的 `setBlock`／tick 呼叫重播到活的世界） | `tree_parity` 129／129 |
| wp44-interact | 告示牌（編輯、編輯鎖、染色、上蠟、螢光墨囊、點擊事件、放置）、書與筆、右鍵穿裝備、中鍵挑選、挖礦經驗（`spawnAfterBreak`／`popExperience`） | `interact_parity` 409 個 scenario＋挑選表 35,723 個方塊狀態；loot 的 `block_experience` |
| wp44-combat | `Player.attack` 搬進 `melee.rs`：玩家對生物與玩家、橫掃、暴擊、重錘（smash、density、breach、wind burst 與擊退爆炸）、護甲與附魔、騎乘、水中 | `melee_parity` 690／690 |
| wp44-player | 玩家自己的身體（`phantom.rs`）：兩個封包之間的重力與阻力、`applyEffectsFromBlocks` 的 step 收集器、摔落傷害與落地方塊、仙人掌／甜莓／凋零玫瑰／粉雪／窒息、凍傷、`InhabitedTime` 以外的玩家環境 | `effect_parity` 651／663 |
| wp44-spawner | `BaseSpawner`（刷怪磚）、洞穴蜘蛛、自然生成的結構覆寫表、攀爬生物不可被推 | 49＋23,073 個樣本 |
| wp44-admin | 世界 seed、難度與鎖、遊戲規則、`ops.json`、command storage 的讀寫與原版互載；區塊 `InhabitedTime`；登入封包帶 hardcore、reduced debug info、死亡畫面、limited crafting 與雜湊 seed；`spawn_phantoms`、`spawner_blocks_work`、爆炸掉落衰減等規則接線 | `admin_check.py` 27／27 |
| wp44-end | 末影之眼與終界傳送門框架、`/locate structure`、`/place structure|jigsaw|feature` | 眼 40＋框架 28 個向量、locate 393／393 |

### 6.2 wp45：整合、完成、修正、放棄

完成（存檔時停在半途的部分）：

- 合併順序 wp41 → wp44-parity-audit（含 farming、misc）→ growth → trees → interact → admin → spawner → player → combat → end。衝突都是「雙方各加一段」：`behaviour/mod.rs` 的模組與分派鏈、`TestLevel` 的欄位、`BlockTickVectors.java` 的 scenario 清單與 `runScenario`、`MobVectors.java` 的輸出欄位、`blocks.rs` 的 region 部件；唯一語意衝突是 wp41 的火球偏轉與 wp44-combat 搬走的 `Player.attack`，偏轉移進 `melee::attack`（`deflectProjectile` 在 `onAttack` 之後、傷害之前）。
- 重錄所有受影響的向量並重放：方塊 314 個（185＋129）、interact 409、melee 690、矛 148、效果 663、實體 1,241、結構生怪、wp41／wp44 的生物向量併入 `m6-mobs2`（867 → 988 個 scenario，457,518 個狀態）。
- 守衛的缺口：`command_diff.py` 1,956 行全相符、`--structures` 393／393、`admin_check.py` 27／27。

修正（半成品的實際錯誤，皆由向量或測試抓到）：

- harness 的洩漏：`InteractVectors` 的使用統計跨 scenario 累加（玩家統計以 uuid 保存，改成每個 scenario 記差值）；`BlockTickVectors` 先跑倒水的 scenario，底層虛空的水池幾百 tick 後漫到視窗邊緣，污染一個 1,000 步的長 scenario（長的先跑）；`EffectVectors` 的仙人掌放在石頭上（Kiln 的作物排程 tick 會讓它掉，要放在沙上、沙下再墊石頭）；重放端的玩家身體帶著上一段的速度（`server_delta`，先站過地面的玩家在空中第一 tick 就多動一格，粉雪與摔落共 30 多個 scenario 差一 tick）、重放世界裡的自然生成（史萊姆區塊的史萊姆被當成受害者）、矛的重放世界沒有地板（身體會掉，擊退的 y 分量因此為 0）。
- `kiln-entity` 的 `isSuffocating`：原版的預設判斷是「在 `#causes_suffocation` 且碰撞為整格」，擷取時 tag 還沒載入，石頭永遠是 false，玩家與生物卡在牆裡不會受傷。改為擷取「是否用預設」（`suffocating_default`）再在執行期查 tag（`physics.bin` 已重產）。
- 指令載入的方塊實體資料把預設欄位（告示牌兩面空白文字）重複塞進去而不是取代；載入的告示牌資料現在存成完整形式（顏色、發光、四行）。
- 手寫書（書與筆）使用時不送 `open_book`（只有成書才送）；懸掛告示牌貼在方塊側面時，朝向要垂直於被點擊面的軸（`WallHangingSignBlock.getStateForPlacement`），原本沿著該軸。
- 守衛者的荊棘傷害沒有把玩家擊退（`Event::Hurt` 沒帶攻擊者）；`Sim::inventory` 的選單視圖沒有裝備欄（也是 state hash 的一部分）。
- 測試：`clientbound_golden`（登入封包多了 hardcore、reduced debug info、死亡畫面、limited crafting 和雜湊 seed，golden 用 `packet_vectors.py --bless` 由原版重新驗證）、`crowd_golden`（四組常數重錄，`verify_locator_bar` 仍比對最佳化與直接版）、`tests/interact.rs`（期望值是錯的：格式碼只去掉 `§c` 整個、頭盔不堆疊）、`items.rs`（超平坦世界沒有草的骨粉）、`sculk.rs`（玩家要站上半格高的嚎叫者）、`spawners.rs`（關掉自然生成）、`trees.rs`（等真實時間的區塊生成）、`kiln-world` 的告示牌預設資料。

放棄（不值得完成，已在矩陣標成 D）：

- 創造模式 Ctrl＋中鍵把方塊實體資料帶進物品（`block_entity_data` 元件與 `collectComponents`），以及橫幅與裝飾陶罐的圖樣：只有創造模式用得到，向量裡這幾個 scenario 拿掉了。
- `kilndiff` 測試包裡七個沒有任何指令用到的 predicate 檔（兩個在 26.3 解析失敗，讓原版起不來）。
- `effect_parity` 的 12 個差異（3.3 表後）：保留在向量裡並以 `KNOWN_GAPS` 把關，沒有為了湊綠而刪掉。
- 超平坦世界的特徵宿主（樹苗與草的骨粉在 `KILN_GENERATOR=noise` 才長）。

### 6.3 wp49（`wp49-d-gaps`）：補 D 項

目標：把第 4 節仍是 D 的項目做到與原版 26.3 一致，並用原版錄製的向量驗證。做法同 wp44／wp45：先用 `javap -c -p` 讀原版的反編譯碼，用最簡單而精確的版本實作，再由 `tools/*Vectors.java` 在原版伺服器內跑場景、Kiln 重播逐項比對。新向量放在 `work/wp49/`（`mobs`、`interact`、`container`、`block`、`entities`、`explore`、`initial`），重放入口都寫進 `tools/parity_suites.py`（`container49`、`interact49` 對目錄下每個檔各跑一次；生物併入 `m6-mobs2`）。

| 項目 | 內容 | 原版向量 | 驗證端 |
|---|---|---|---|
| 蜜蜂與蜂巢 | `Bee` 全部 goal（授粉、回巢、憤怒與攻擊、嬰兒）、蜂巢方塊實體（蜂蜜、蜜蜂進出、煙燻、剪取）、蜂巢互動 | 蜜蜂 30、蜂巢互動 22、蜂巢容器 6 | `mob_parity`、`interact_parity`、`container_parity` |
| 海豚、巨人、快樂恐懼魔 | `Dolphin`（全部 goal 與換氣、躍出）、`Giant`、`HappyGhast`（成長、harness、呼吸、跟隨）；`ForNonPathfinders` goal 與年齡邊界掛鉤 | 27／4／20 | `mob_parity` |
| 銅傀儡 | 在箱子之間搬運（`TransportItemsBetweenContainers`：來源銅箱、目的普通箱、開關音效與開啟者計數）、風化與雕像、蠟與斧、雷擊、與鐵傀儡的送花（`OfferFlowerGoal`）、銅塊＋南瓜建造；雕像方塊的姿勢切換與斧 | 34＋雕像互動 14 | `mob_parity`、`interact_parity` |
| 硫磺方塊 | 12 種原型（彈性、摩擦、空氣阻力、爆炸擊退抗性、接觸傷害、是否浮在液體上）、吞物與吐出、餵食與分裂、TNT 原型的引信與爆炸、水中的行為；新屬性 `bounciness`、`explosion_knockback_resistance` | 29 | `mob_parity` |
| 資料實體 | `block_display`、`item_display`、`text_display`（含選擇器與分數解析）、`interaction`、`marker`、畫、展示框與發光展示框、盔甲座 | `entity_nbt` 127（122 逐欄相同、5 個留給模擬端）、展示框 55、盔甲座 142 | `entity_nbt.rs`、`interact_parity` |
| 方塊實體與方塊 | 營火烹飪、蜂巢、鐘、講台、裝飾陶罐、雕紋書架、合成器、製圖台與地圖（`MapItemSavedData`、探索地圖、旗幟、展示框地圖）、試煉刷怪磚、寶庫、指令方塊（普通、連鎖、重複、條件式、礦車）、日光感測器、蛋糕與蠟燭蛋糕、刷子與可刷方塊、氣泡柱 | interact 799（`interact49` 18 檔）、container 247（`container49` 12 檔）、bubble 6、explore 11 | `interact_parity`、`container_parity`、`block_parity`、`exploration_map_parity` |
| 發射器 | 骨粉、打火石、蜂蜜與玻璃瓶、發光石、TNT、潛影盒、船、礦車、盔甲座、全部拋射物（箭、藥水箭、光靈箭、雪球、蛋、藥水、經驗瓶、煙火、火焰彈、風彈）、水／岩漿／粉雪桶與生物桶（魚、蠑螈、蝌蚪、硫磺方塊）、生怪蛋、南瓜與凋零頭顱的傀儡建造、穿裝備（盔甲座、玩家、拾取戰利品的生物、馬鞍與馬鎧、熾足獸鞍）、箱子上驢羊駝、硫磺方塊吞物、剪雪人／哞菇／羊／bogged | 100 | `container_parity`（`dispenser`） |
| 玩家風彈 | `minecraft:wind_charge` 實體（半徑 1.2、擊退乘 1.22、5 tick 內不可被偏轉）、`WindChargeItem.use`、發射器射出；爆炸的 `explosion_knockback_resistance`；拋射物能打中礦車、船、盔甲座、畫與展示框、火球與風彈（`Entity.canBeHitByProjectile`） | `wind_` 27 | `container_parity`（`track`：每 tick 比對所有實體的位置、速度、血量） |
| 霜行者、靈魂疾行者 | 附魔的 `location_changed`（換方塊或落地時：靈魂疾行者的速度與移動效率修飾子，疊在靈魂沙／土上，靴子磨損；霜行者在腳下半徑 3＋(等級−1) 的水源上鋪霜冰，不騎乘、在地上才鋪）與 `tick`（靈魂粒子與音效）；霜行者對熱地板的傷害免疫本來就由戰利品引擎處理；新屬性 `movement_efficiency` | `ench_` 17 | `effect_parity`（`effects49`） |
| 村莊圍攻、貓生成 | `Siege`、`CatSpawner`（沼澤小屋的黑貓） | 無（原版錄製不可行） | 單元測試（C） |
| 玩家姿勢、氣泡柱、封包移動 | `Player.updatePlayerPose`（游泳、爬行、強迫蹲）、`aiStep` 的 0.003 速度歸零、氣泡柱推玩家、封包移動走伺服器身體（`player::server_move`） | `effect_parity` 663／663 | `effect_parity`（沒有已知差異，見 3.3） |
| 初始動物 | 區塊產生時的動物（群組大小、`isValidSpawn` 逐種條件） | 7 個世界、各 169 個區塊的窗口（161 隻中 114 隻逐位元相同，其餘在原版順序相依的容許度內） | `tests/initial_mobs.rs` |
| 命名牌 | 命名牌命名生物；自訂名稱、靜音、無重力送給觀看者的實體資料 | `tests/` 單元測試 | C |

這一輪找到並修掉的 Kiln 錯誤（皆由向量或新測試抓到）：

- **只有旁觀者的 region 會把非持久的怪物立刻清掉**：`Level.getNearestPlayer` 不計旁觀者（沒有玩家 → 不清），Kiln 的 `any_player` 把旁觀者也算成「有玩家但很遠」，剛生成的怪物當場被當成過遠而 `discard`。這個錯誤蓋住了另一個（試煉刷怪磚的 `trial_spectator_player` 向量的重放端在設好遊戲模式前先讓生存模式玩家站了一個 tick）；兩個一起改了。
- 發射器、投擲器、觀察者、活塞、木桶、指令方塊與合成器放置時朝向玩家看的方向（`getNearestLookingDirection`），不是被點的面。
- 箭、光靈箭、藥水箭從發射器射出時可被撿起（`pickup = ALLOWED`）；之前是不可撿。
- 盔甲座落下的阻力用 `0.98f`（float）；之前用 double。
- 容器向量的重放依賴 datapack（方塊被更新打掉時的戰利品表）與語言檔（`KILN_LANG`：指令方塊的最後輸出是原版的英文句子）；`parity_suites.py` 兩者都設了，單獨跑 `cargo test` 要自己設。

效能（`sim_load --players 300 --groups 6 --ticks 600`，噪音地形、資料包開，同一台 VM 輪流跑）：wp49 前（`d77e20b9`）每 tick 平均 1.74～1.85 ms，wp49 後 2.03～2.13 ms（狀態雜湊兩者相同，`6354d84ec39048c6`），目標 5 ms 之內。多出的約 0.3 ms 來自原版本來就要做的事：伺服器端身體對每個移動封包做碰撞（`server_packet_move`，約 0.1 ms）、每 tick 的姿勢判斷（`update_pose`，約 0.1 ms，已改成只查一次方塊）、附魔位置效果與方塊實體的每 tick 檢查。量測時抓到並修掉的三處浪費：沒有任何地圖資料時不掃實體與玩家（原本每 tick 0.055 ms）、發射器要用的「穿裝備資訊」只替發射器附近的實體算、姿勢判斷在想要的姿勢放得下時少查一次碰撞。

仍是 D 或 C 的（以及原因）：

- **實體**（wp50 補上）：`mannequin`、`cushion`、`ominous_item_spawner`。
- **豬鞍與胡蘿蔔釣竿、鸚鵡螺鞍與護甲**（wp50 補上）。
- **發射器**：刷子對犰狳與剪斷拴繩在 wp50 補上；穿裝備時的裝備音效與遊戲事件、拾取戰利品的生物的 `canPickUpLoot` 隨機性只靠生物自己的值。
- **無法用原版向量驗證的**：貓生成與村莊圍攻（原版的亂數與計時不可重播，只有 Kiln 單元測試）；硫磺方塊的玩家推擠、衝刺擊退與接觸傷害對生物（harness 不 tick 玩家）；初始動物的順序相依（原版的結果依區塊產生順序，以量測到的容許度比對）。
- 與 wp45 相同、這一輪沒碰的：配方書封包位元組、釀造台運作向量、選單點擊向量補鐵砧等（「moved wrongly」、結構方塊與拼圖方塊、旗幟圖樣在 wp50 補上）。

### 6.4 wp50（`wp50-last-d`）：做完剩下的 D 項

目標：把 wp49 之後仍是 D 的項目做完，並用原版 26.3 錄的向量驗證。做法同 wp49（讀 `javap -c -p` 的原版碼、最簡單而精確的實作、`tools/*Vectors.java` 在原版伺服器內跑場景、Kiln 重播逐項比對）。向量在 `work/wp50/`（`mobs`、`interact`、`container`、`inventory`、`entities`），`tools/parity_suites.py` 的 `interact50`（`wp50/interact/*.jsonl` 全部）、`container50`、`loom_clicks`、`ominous_trial` 把它們接進整套；生物向量（`eq50_*` 68 個）併入 `work/m6-mobs2/vectors.jsonl`（原檔備份為 `.pre-wp50`，依 name 去重後 1,227 行，`mob_parity` 1,198 個 scenario 通過）。

| 項目 | 內容 | 原版向量 | 驗證端 |
|---|---|---|---|
| cushion | `Cushion` 實體：`BlockAttachedEntity` 的存活檢查（每 100 tick，post-increment）、顏色與自訂名稱、坐上與坐下音效、離開、被雷劈（四種附著實體共用修正）、`/summon` 與存檔 | 152 | `interact_parity`（`cushion50`） |
| mannequin | `Mannequin`（`LivingEntity` 而非 `Mob`）：屬性只有生物共通的一組、`profile`／`hidden_layers`／`main_hand`／`pose`／`immovable`／`description` 存檔、`immovable` 時不 travel、26.3 的 living 存檔欄位；命名牌只對非 mob 生效；火與岩漿傷害在 tick 內延後 | 行為 46、NBT 53 | `interact_parity`（`mannequin50`）、`entity_nbt.rs` |
| ominous_item_spawner | 不祥試煉刷怪磚的物品雨（物品實體、拋射物種類、延遲）、骨頭與空堆疊 | 15 | `container_parity`（`ominous50`）、`tests/ominous_trial.rs` |
| 豬、地獄疣豬行者的鞍與胡蘿蔔釣竿；鸚鵡螺鞍與護甲 | 鞍欄位、放鞍（右鍵與發射器）、剪下、騎乘與操控（含胡蘿蔔釣竿加速）、AI 在有鞍時的行為；鸚鵡螺與殭屍鸚鵡螺的鞍與護甲 | 68（`eq50_*`，含馬、驢、羊駝、駱駝的對照組）＋發射器放鞍與裝備 21（`dispenser_equip50_*`） | `mob_parity`、`container_parity` |
| 發射器的刷子與剪刀 | 刷子對犰狳（含幼體、已刷過、別種生物、什麼都沒有）、剪刀剪斷拴繩（`shearOffAllLeashConnections`） | `container50` 的 `c50` 中 `dispenser_brush50_*` 5、`dispenser_shears50_*` 4 | `container_parity` |
| 旗幟圖樣 | 織布機選單（按鈕順序來自 tag 的 JSON 順序、`slotsChanged` 對任何來源重算）、盾牌上旗幟、鍋釜洗旗與洗盾 | 選單點擊 1,500 序列／35,988 步、`banner50` 19、`cauldron50` 170 | `kiln-inventory/tests/click_parity.rs`、`interact_parity` |
| 結構方塊 | 螢幕封包（更新資料、存範圍、載入範圍、偵測大小）、模式（存、載入、角落、資料）、範本庫（先找世界 `generated/`，再找 datapack 與遊戲資料；找不到的也記住）、`fillFromWorld`（略過結構空位、帶方塊實體資料）、`placeInWorld`（旋轉、鏡射、完整度的 `BlockRotProcessor` 與種子、`strict` 的旗標 816、方塊實體載入並依模式更新方塊）、紅石觸發（存、載入、角落卸載）、放置者為作者、`StructureTemplate` 預設作者 `?`、`DataVersion` 與 `id`／`properties` 的存檔鍵 | `structure50` 88 | `interact_parity`（含範本 NBT 逐位元組比對） |
| 拼圖方塊 | 螢幕封包、方塊實體欄位與 `joint` 預設（依朝向）、放置朝向（`getStateForPlacement`）、`JigsawGenerate` 接到 `/place jigsaw` 的機制 | 含在 `structure50`（9 個 jigsaw scenario） | `interact_parity` |
| `haz_snow_lava_clears` | 效果向量唯一的已知差異：查明是錄製端假象（影子用戶端先融掉粉雪），`EffectVectors` 加 `quietShadow` 重錄，`wp45/effects/vectors.jsonl` 內該行換新（原檔備份為 `.pre-wp50`），`effect_parity` 663／663 | `effect_parity` 的 `KNOWN_GAPS` 清空 | `effect_parity` |
| 「moved wrongly」 | 封包移動先讓伺服器身體走 `player::server_move`，水平距離平方超過 0.0625（創造、旁觀、睡眠不計；垂直分量被原版的比較式永遠歸零）且舊位置沒有碰撞，或新位置撞到新的碰撞形狀，就退回起點；被退回的移動從方塊效果的移動紀錄中移除；潛行不會走下邊緣的身體退讓是這條檢查的主要來源 | `moves50` 27 | `interact_parity` |

這一輪找到並修掉的 Kiln 錯誤（皆由向量抓到）：

- 織布機：背包裡有束口袋時 `slotsChanged` 也要重算；按鈕順序不是字母順序而是 tag 的 JSON 順序。
- `BlockAttachedEntity` 的存活檢查在第 100 次之後才做（`ticksSinceLastCheck++ >= 100`），畫、展示框、拴繩結一起改。
- 站上方塊就移走的標誌牌：standing／wall sign 在放置範圍的邊緣依 `updateShape` 掉落（`StructureTemplate.updateShapeAtEdge`），和旗幟走同一條規則。
- 結構方塊／拼圖方塊放置時的方塊狀態（`JigsawBlock.getStateForPlacement`）、`loadAdditional` 的 `updateBlockState`（模式改了方塊跟著改）。
- 資料指令改方塊實體後，區塊副本與「原版解析後再存出」一致（箱子沒有戰利品表就不留 `LootTableSeed`）。
- 閃電 `ext_thunder_hit`：不是延伸實體的生物（例如被劈中的豬變成的殭屍豬人）在檢查前就被換成佔位狀態，資料遺失而從世界消失；`tests/weather.rs` 的 `lightning_charges_creepers_converts_pigs_and_lights_fire` 抓到，先檢查再換。

仍是 D 或 C 的（以及原因）：

- **結構方塊存檔不收集範圍內的實體**（`fillEntityList`）：wp52 補上（6.5）。
- **拼圖方塊 `generate` 的 `keepJigsaws`**：wp52 接上（6.5）；結構範本放置需要 datapack 的噪音設定才能建立區域（沒有時回報失敗）。
- **Kiln 在結構方塊流程中會多送中間狀態的方塊實體封包**：wp52 修掉（6.5）。
- **cushion**：wp52 補了流體（岩漿）與中鍵挑選，活塞推動仍未比對；**mannequin**：生物落地的方塊音效 harness 不錄（比對時濾掉）。
- **「moved wrongly」**：被退回的移動的 `doCheckFallDamage` 與重錘後寬限由 wp52 補上（6.5）；身體卡在蜘蛛網等方塊的第二步，原版 harness 不 tick 玩家所以沒有向量（Kiln 在 tick 內套用）。
- 還沒做（wp52 已做，見 6.5）：`player_sheared_equipment` 的進度 trigger、狼鎧甲、生怪蛋生出幼體。

### 6.5 wp52（`wp52-gaps`）：關掉剩下的缺口，並補外掛 API 1.1

目標：把 6.4 結尾「仍是 D 或 C」的項目能做的做完，每一項先讀原版（`javap -c -p`）、在原版伺服器內錄向量、Kiln 重播逐項比對。向量在 `work/wp52/`（`interact`、`container`、`combat`、`effects`、`mobs`），`tools/parity_suites.py` 新增 `interact52`（`wp52/interact/*.jsonl`）、`container52`、`melee52`、`effects52`；生物向量併入 `work/m6-mobs2/vectors.jsonl`（原檔備份為 `.pre-wp52`，依 name 去重後 +114 個 scenario，`mob_parity` 共 MOB_TOTAL 個 scenario 通過）。

| 項目 | 內容 | 原版向量 | 驗證端 |
|---|---|---|---|
| 結構方塊存檔收集實體 | `fillEntityList`（`includeEntities`）：範圍內非玩家實體、乘客與「不存檔」實體只留空 nbt、畫的 `block_pos` 改成相對座標、依區段排序；載入端的掛飾（展示框、畫）依旋轉與鏡射轉向（`HangingEntity.rotate/mirror`，座標取 floor） | `structure52` 4（帶實體存檔、`includeEntities` 關、區段排序、載入） | `interact_parity` |
| 結構方塊流程的方塊實體封包 | 載入資料時方塊變更先保留（`with_level_held`、`defer_be_packet`），方塊實體封包只在最終資料出去一次；`structure50` 不再合併同位置封包，88／88 逐封包相符 | `structure50` 88 | `interact_parity` |
| 拼圖方塊 `keepJigsaws` | `PoolElementPiece.keep_jigsaws` 一路接到 `generate_jigsaw`；`/place jigsaw` 恆為 false（原版同），結構方塊的 `JigsawGenerate` 封包帶旗標 | 無（原版 `/place jigsaw` 不能設 true；封包路徑需要完整 UI） | C |
| 狼鎧甲 | 穿上（主人、成狼、沒穿）、傷害吸收與耐久（`bypasses_wolf_armor` 不吸收、`hurtAndBreak(ceil)`）、裂痕等級 0.95／0.69／0.32 的音效與 20 顆犰狳鱗片粒子、破裂事件 65、剪刀拆下、犰狳鱗片修理（坐著＋主人＋受損，`max_damage/8`）、染色（既有）、死亡掉落（保證）、發射器裝備、存檔 `equipment.body`／`drop_chances.body` | 狼 `eqwolf52_*`／`hurtwolf52_*` 70（併入 `m6-mobs2`，含先前未錄的 `eqwolf50_*`）、近戰 `melee52` 53、發射器 `c52` 5 | `mob_parity`、`melee_parity`、`container_parity` |
| 生怪蛋對同種生物 | `SpawnEggItem.spawnOffspringFromSpawnEgg`：同種生物（成體）使用生怪蛋生出幼體（命名牌後；自訂名稱、殭屍加權旗標、狐狸信任玩家、豬布林與豬靈獸的幼體設定；鸚鵡、流浪商人不生）；生怪蛋對刷怪磚／試煉刷怪磚的既有路徑不變 | `egg52` 44（貓熊的子代隨機數無法重播，標 `diverges`） | `mob_parity` |
| 進度 trigger | `player_sheared_equipment`（剪刀剪下穿戴裝備）、**`target_hit`（靶方塊進度原本永遠不會觸發，是 Kiln 的錯誤）**、`fall_after_explosion`、`crafter_recipe_crafted`（合成器丟出成品時，17³ 內玩家）、`voluntary_exile` 的解析、`thrown_item_picked_up_by_player`（玩家撿起被別的實體丟出的物品，如悅靈送物） | 沒有（trigger 只有 Kiln 這一側的接線，沒有原版進度向量） | C |
| 衝擊上下文與重錘寬限 | `currentImpulseImpactPos`／`currentImpulseContextResetGraceTime`（40 tick）、`causeFallDamage` 扣掉爆炸高度、`fall_after_explosion`、風彈爆炸與重錘 smash 設定、重錘擊退的 `onExplosionHit`、存檔鍵 | `impulse` 24（`effects52`） | `effect_parity` |
| 「moved wrongly」補完 | 被退回的移動做 `doCheckFallDamage(0,0,0,onGround)`；衝擊寬限期間不判「moved wrongly」 | `moves52` 16 | `interact_parity` |
| 實體中鍵挑選 | `handlePickItemFromEntity`／`getPickResult`：生物→生怪蛋、盔甲座、終界水晶、畫、拴繩結、展示框（有物品則是物品）、cushion、礦車、船；距離檢查（互動距離 +3）；生存模式只換已有的 | `pickent52` 44 | `interact_parity` |
| cushion 岩漿 | 岩漿在 cushion 的方塊內會燒它（含音效）；流體在密閉石箱內逐 tick 比對 | `cushion52` 13 | `interact_parity` |
| 物品元件→方塊實體 | **`block_state` 元件在放置時被忽略（Kiln 錯誤）**，現在照 `updateBlockStateFromTag` 套用；頭顱的 `profile`／`custom_name`／`note_block_sound`、附魔台 `CustomName`，不可命名的方塊實體（終界箱、告示牌）的名稱留在 `components` | `place52` 55 | `interact_parity` |

這一輪找到並修掉的其他 Kiln 錯誤（皆由向量抓到）：創造模式對生物使用物品（`HeldChange::Shrink`）不應扣物品；礦車的 `HasTicked` 沒存檔／讀取；掛飾放置時沒有依旋轉轉向；結構範本放置的實體 `Pos` 與 `block_pos`。

仍是 C 或 D 的（以及原因）：

- **進度 trigger 還缺**（wp53 已接，見 6.6）：`spear_mobs`（長矛 `KineticWeapon` 的命中數）、`bee_nest_destroyed`（絲綢之觸蜂巢，`BeehiveBlock.playerDestroy` 沒有玩家事件的接線）、`thrown_item_picked_up_by_entity`（猴子／豬布林撿起玩家丟的金錠，`distract_piglin`、`uh_oh`）、`allay_drop_item_on_block`、`used_ender_eye`、`any_block_use`／`default_block_use`（沒有原版進度用到）。已接的 trigger 沒有原版向量。
- **放置物品的方塊實體預設**（wp53 已修，7 個 scenario 放回，見 6.6）：漏斗 `TransferCooldown`（原版不 tick 為 -1，Kiln 放置後 tick 一次）與釀造台 `total_brew_time`／`total_fuel`（原版新建為 0／0，Kiln 載入預設 400／20）、刷怪磚 `SpawnPotentials` 預設（原版 `[]`）與「op 帶 `block_entity_data` 的刷怪磚物品」（目前只有指令方塊吃 `block_entity_data`）：這 7 個 `place52_*` scenario（`hopper_named_*`、`brewing_named_*`、`spawner_data_*`、`op_spawner_data`）從向量檔移除，沒有修。
- **鎧甲被「撿起」**（wp53 已做，見 6.6）：持有 `CanPickUpLoot` 的生物不會把發射器丟出的狼鎧甲撿到身上（`c52` 該 scenario 已拿掉）。
- **cushion 活塞推動、mannequin 落地方塊音效**（wp53 查過：harness 其實可以 tick，缺的是 Kiln 的功能，見 6.6）：harness 不 tick 實體／不錄落地音效，無法比對（Kiln 有實作，沒有向量）。
- 命令回饋的 `show_entity` 懸停事件 Kiln 沒有送（`kill` 之類的回饋）；向量因此避開。
- 實體 NBT 差異：豬的 `attributes` 與雞的 `variant` 的存檔欄位和原版不同（向量改用礦車／盔甲座場景）。

外掛 API 的 1.1 增補見 `docs/plugin-api.md` §10。

### 6.6 wp53（`wp53-gaps`）：關掉 wp52 留下的缺口，並用 1.0 guest 驗證外掛 API 的相容

方法照舊：原版 javap 讀行為、原版內錄向量、Kiln 重播；沒辦法錄的就說沒辦法。向量在 `work/wp53/`（`interact/adv53.jsonl`、`container/c53.jsonl`、`mobs/pickup53.jsonl`），
`tools/parity_suites.py` 新增 `interact53`、`container53`、`plugin_compat`；wp52 的 `place52.jsonl` 補回 7 個 scenario（62 個），生物向量併入 `work/m6-mobs2/vectors.jsonl`
（原檔備份為 `.pre-wp53`，依 name 去重後 +34）。

| 項目 | 內容 | 原版向量 | 驗證端 |
|---|---|---|---|
| 進度 trigger | `bee_nest_destroyed`（`BeehiveBlock.playerDestroy` 只要玩家用得上掉落就觸發，**絲綢之觸也觸發**；`num_bees_inside` 是破壞後還留在方塊裡的蜂數）、`used_ender_eye`（`EnderEyeItem.use`：玩家到最近要塞的水平距離平方，`matchesSqr`）、`spear_mobs`（`KineticWeapon.damageEntities` 有命中時，近期刺過的活生物數 ≥ `count`）、`thrown_item_picked_up_by_entity`（`LivingEntity.onItemPickup`：玩家丟的物品被生物撿起；接在豬布林、硫磺方塊、悅靈、狐狸、貓熊、海豚與新的通用撿拾）、`allay_drop_item_on_block`（`AllayAi.throwItem`：喜歡的玩家、目標下方的方塊、丟出的那一個物品）、`any_block_use`（`handleUseItemOn`：凡是 `consumesAction` 的點擊，帶手上剩下的物品）、`default_block_use`（空手 `useWithoutItem` 被方塊接走） | `adv53` 13（`husbandry/silk_touch_nest`：蜂巢／蜂窩 × 0／2／3 隻蜂 × 絲綢／非絲綢 + 創造模式；只有「絲綢＋3 隻＋蜂巢（nest）」完成準則） | `interact_parity`（InteractVectors 現在可以 `advancements()` 記錄每一步完成的準則）；其餘 6 個 trigger 沒有原版向量，見下，由 `advancements/wp53_tests.rs` 6 個測試覆蓋 |
| 放置物品的方塊實體預設 | **新建**的釀造台 `total_brew_time`／`total_fuel` 是 0／0（`loadAdditional` 的 400／20 只給缺這兩個鍵的存檔）；**新建**的刷怪磚沒有 `SpawnData`、`SpawnPotentials` 是 `[]`，且它的存檔欄位立刻寫進區塊副本（更新封包有 `Delay`、`MaxNearbyEntities`… 而不是空 tag）；`BlockItem.updateCustomBlockEntityTag` 對刷怪磚：只有 `canUseGameMasterBlocks` 的玩家，`block_entity_data` 與存檔合併（`CompoundTag.merge`）後讀回；漏斗 `TransferCooldown` -1 本來就對（原版未 tick 的區塊副本是 -1，Kiln 的重播多 tick 一次到 0），所以那兩個 scenario 改錄成「關卡有 tick」（`ticking`）。**這是 wp52 的「放置後 tick 一次」差異，不是 Kiln 的錯誤** | `place52` 55 → 62（hopper_named、brewing_named、spawner_data ×2 模式 + op_spawner_data） | `interact_parity` |
| 持有 `CanPickUpLoot` 的生物撿東西 | 新模組 `mob/pickup.rs`：`Mob.aiStep` 的撿拾迴圈（1×0×1 範圍、`mob_griefing`）→ `wantsToPickUp` → `pickUpItem` → `equipItemIfPossible`（`getEquipmentSlotForItem`、`isEquippableInSlot`、`canReplaceCurrentItem`：護甲看護甲值與韌性、綁定詛咒不換、武器看偏好武器 tag 與攻擊傷害、`canReplaceEqualItem`；護甲不比較好就改放空的主手；舊的依掉落機率掉出、一次穿 1 件、`onEquipItem` 的音效種子）；`Zombie`／`ZombifiedPiglin`／`Drowned`／`AbstractSkeleton`／`WitherSkeleton` 的 `canHoldItem`／`wantsToPickUp`／偏好武器；狼的身體護甲（BODY 槽）與豬的鞍都走這條。有自己撿法的（豬布林、狐狸、貓熊、海豚、悅靈、村民、襲擊者、硫磺方塊）不經過它。順手修了 `wither_skeleton` 不在 `#burn_in_daylight` 的錯（它免火但仍抽 `isSunBurnTick` 的亂數） | `pickup53` 34（殭屍／屍殼／溺屍／殭屍豬人／骷髏／凋零骷髏／牛／豬／苦力怕／狼 × 護甲、武器、綁定、矛、發光墨囊、鞍、狼鎧甲）；`c53` 1（發射器丟狼鎧甲、穿著鎧甲的狼自己撿進嘴裡）| `mob_parity`、`container_parity`（`mobs_lag`：發射器在方塊階段做的物品，Kiln 在下一個 tick 才被撿起，原版同 tick——和 `drops_lag` 同一個架構差異） |
| 外掛 API 1.0 guest 在 1.1 host 上跑 | `tests/fixtures/compat10`：直接以凍結的 1.0 WIT 建置的 guest（70 KB `compat10.wasm` 進版控，附原始碼與 `build.sh`）；`tests/api_compat.rs` 載入並呼叫它。**第一次跑就失敗**：1.1 在 `observed` 尾端加了 `player-moved`，1.0 guest 的 region 實例化被 wasmtime 以 `expected variant of 5 cases, found 4 cases` 拒絕。改成新的選用匯出 `move-hooks.on-moved`（WIT、host、SDK、`lockbox`），`wit_freeze.rs` 新增逐宣告檢查，把「1.0 的宣告必須一字不差地還在」變成測試 | – | `api_compat`、`wit_freeze`（`plugin_compat` 套件） |
| `get-block` 視窗的複製成本 | 7012 ns／事件 → 每個 chunk 只找一次後 **3709 ns／事件**（729 個方塊，5.1 ns／方塊），逐方塊與舊作法比對相同 | – | `plugins::tests::block_window_copy_cost` |

仍是 C 或 D 的（以及原因）：

- **進度 trigger 的原版向量**：只有 `bee_nest_destroyed` 有。`any_block_use`、`default_block_use`、`used_ender_eye` 沒有任何原版進度用到（要錄就得在原版伺服器放自訂 datapack，Kiln 的重播也要載同一份；沒做）。`spear_mobs`
  （`adventure/spear_many_mobs`）需要玩家蓄力刺中 5 隻會動的生物，InteractVectors 不 tick 生物；`thrown_item_picked_up_by_entity`（`nether/distract_piglin`、`husbandry/uh_oh`）與 `allay_drop_item_on_block`
  需要 tick 生物並記錄玩家的進度，這是 MobVectors 的範圍，而 `mob_parity` 在 kiln-entity 裡沒有進度系統可比對（只能比事件發出的 tick，不比條件）。這 5 個 trigger 的條件判斷有單元測試，
  生物端的事件只有「發出」沒有與原版比對 tick。
- **村民與襲擊者的非旗幟撿拾**：村民（`Villager.pickUpItem`／`wantsToPickUp`，只撿食物與種子進背包）與襲擊者（`Raider.pickUpItem`、掠奪者的 `wantsItem`）在 Kiln 沒有撿起一般物品的路徑，通用撿拾也跳過它們。
- **撿拾不送 `take_item_entity` 封包**：生物撿起物品時原版送 `ClientboundTakeItemEntityPacket`（客戶端看到物品飛向生物），Kiln 的生物撿拾（包含豬布林）只讓物品消失。
- **cushion 活塞推動、mannequin 落地方塊音效**：**不是 harness 的問題**。InteractVectors 從 wp50 起就有 `tick_cushions`（每步 tick 一次，或 `ticks` 次）並 tick mannequin，封包也錄音效；缺的是 Kiln 本身：
  (1) 活塞根本不推實體——`PistonMovingBlockEntity.moveCollidedEntities` 沒有實作（玩家、生物、掉落物、cushion 都不會被活塞推或夾），cushion 被推時該有的 `BlockAttachedEntity.move(PISTON)` 破壞也就沒發生；
  (2) Kiln 沒有任何方塊 `SoundType` 的表（`kiln-data` 沒有），所以玩家與生物的 `LivingEntity.playBlockFallSound`（摔傷時的方塊落地音）、方塊腳步聲、放置音都不會出——重播時 `no_pitch` 的 scenario 乾脆略過所有 `block.*` 音效。
  兩者都不是「便宜地擴充 harness」能解決的，要先做成一個子系統（活塞推實體要和原版逐 tick 比對碰撞與位移；方塊音效要先從 `Blocks.java` 抽出 `SoundType` 表）。
- 命令回饋的 `show_entity` 懸停事件、實體 NBT 的兩處存檔欄位差異（見 6.5）。

外掛 API 的修正見 `docs/plugin-api.md` §5、§10。


## 7. 重跑

```sh
# 全部 parity 套件（需 KILN_WORK 指向 work 目錄；KILN_DATAPACK 預設 <work>/generated）
python tools/parity_suites.py [--only mob_parity,fire,...]
# 方塊 hook 盤點：哪些 vanilla 方塊類別有伺服器端行為、Kiln 的原始碼有沒有參照
python tools/parity_audit.py
# 重錄向量（原版伺服器在行程內跑場景；各自一個埠，同時錄時用 KILN_HARNESS_PORT 錯開）
python tools/block_vectors.py --out work/wp45/block/block_vectors.jsonl     # 方塊（前面有長 scenario，勿和別的 harness 同時錄）
python tools/interact_vectors.py --out work/wp45/interact/vectors.jsonl     # 告示牌、書、穿裝備、中鍵挑選
python tools/combat_vectors.py --filter melee --out work/wp45/combat/vectors.jsonl   # 近戰（melee.jsonl）；--filter spear 錄矛
python tools/effect_vectors.py                                              # 效果、摔落、環境傷害
python tools/entity_parity.py --out work/wp45/entity/vectors.jsonl          # 實體（含末影之眼）
python tools/mob_vectors.py --filter "spawner_|cavespider_|push_|avoid_|spider_golem"
python tools/spawn_vectors.py --out work/wp45/spawn/structure_spawns.jsonl  # 結構生怪覆寫（JVM 結束時可能 OOM，檔案已寫完）
# wp49 的向量（直接用 java 單檔執行，cwd 是 <輸出目錄>/server；篩選字串是 scenario 名稱的一部分）
java --add-opens java.base/java.lang=ALL-UNNAMED --add-opens java.base/java.util=ALL-UNNAMED -cp <server jar 與 libraries> tools/ContainerVectors.java work/wp49/container/dispenser.jsonl dispenser_
java ... tools/ContainerVectors.java work/wp49/container/wind.jsonl wind_      # 另有 campfire／hive／lectern／pot／crafter／cmdblock／daylight／target／projectile
java ... tools/InteractVectors.java work/wp49/interact/stand.jsonl stand       # 另有 b49／bell／brush／cake／carto／frames／fulltick／maps／trial／vault／statue…
java ... tools/MobVectors.java work/wp49/mobs/sulfur_cube.jsonl sulfur         # 併入 work/m6-mobs2/vectors.jsonl 時依 name 去重
java ... tools/EffectVectors.java work/wp49/effects/ench.jsonl ench_           # 霜行者與靈魂疾行者
java ... tools/EntityNbtVectors.java work/wp49/entities/nbt.jsonl
java ... tools/ExploreMapVectors.java work/wp49/explore/maps.jsonl 12345       # 第二個參數是 seed
java ... tools/InitialMobVectors.java work/wp49/initial/mobs2.jsonl 2 20      # seed 與半徑；一個世界一個檔，測試用 : 串接
# wp50 的向量
java ... tools/InteractVectors.java work/wp50/interact/structure50.jsonl structure50_   # 另有 cushion50／mannequin50／banner50／cauldron50／moves50
java ... tools/ContainerVectors.java work/wp50/container/c50.jsonl equip50             # 另有 brush50／shears50／ominous50
java ... tools/MobVectors.java work/wp50/mobs/eq50.jsonl eq50_                         # 併入 work/m6-mobs2/vectors.jsonl 前先備份
java ... tools/InventoryVectors.java work/wp50/inventory/loom.jsonl loom
java ... tools/EntityNbtVectors.java work/wp50/entities/mannequin.jsonl mannequin
java ... tools/EffectVectors.java work/wp50/effects/lava.jsonl haz_snow_lava      # 單一 scenario；換進 work/wp45/effects/vectors.jsonl 時依 name 取代整行
# wp52 的向量
java ... tools/InteractVectors.java work/wp52/interact/structure52.jsonl structure52_   # 另有 moves52／pickent52／cushion52／place52（place52 的 hopper／brewing／spawner 已移除）
java ... tools/ContainerVectors.java work/wp52/container/c52.jsonl dispenser_equip50_wolf             # 發射器裝備狼鎧
java ... tools/CombatVectors.java work/wp52/combat/melee.jsonl melee/wolf_armor             # 狼鎧的近戰
java ... tools/EffectVectors.java work/wp52/effects/impulse.jsonl fall_impulse         # 衝擊上下文
java ... tools/MobVectors.java work/wp52/mobs/wolf52.jsonl eqwolf                      # 另有 egg52（eggbaby52_）；併入 work/m6-mobs2/vectors.jsonl 前先備份（.pre-wp52）
# wp53 的向量
java ... tools/InteractVectors.java work/wp53/interact/adv53.jsonl "adv53_.*"        # 蜂巢進度（InteractVectors 的篩選字串是整串比對的 regex，或 name 的一部分）；place52 的 7 個：篩選 "place52_(hopper|brewing|spawner|op_spawner).*"
java ... tools/ContainerVectors.java work/wp53/container/c53.jsonl dispenser_equip50_wolf_armor_on_armored_wolf
java ... tools/MobVectors.java work/wp53/mobs/pickup53.jsonl pickup53_              # 併入 work/m6-mobs2/vectors.jsonl 前先備份（.pre-wp53）；PICKUP_DEBUG=1 印出撿拾條件
# 外掛 1.0 guest：sh crates/kiln-plugin-host/tests/fixtures/compat10/build.sh（要 wasm32-wasip2 target）；cargo test -p kiln-plugin-host --test api_compat --test wit_freeze
# 與原版互載、指令對跑
python tools/admin_check.py --kiln-exe target/release/kiln
python tools/command_diff.py --kiln-exe target/release/kiln --bot-exe target/release/kiln-bot
python tools/command_diff.py --structures --kiln-exe target/release/kiln --bot-exe target/release/kiln-bot
python tools/blocks_diff.py --port 25583        # 45 個 scenario
# 登入等封包的 golden 由原版驗證後寫入
python tools/packet_vectors.py --bless
```
