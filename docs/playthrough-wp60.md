# wp60:第三次真實客戶端遊玩(wp58、wp59 沒走到的部分)

目的:用官方 26.3 客戶端(跑在 VM 的 Xvfb + llvmpipe 上,由 `tools/wp58/ClientCtl.java` 這個 java agent 驅動;客戶端不在 Windows 上跑)
玩 wp58/wp59 沒玩到的東西:比較器鎖定的物品分類器、流浪商人、蜜蜂、馬/驢/騾/羊駝(繁殖、馴服、鞍、拴繩)、地圖(空地圖、製圖台、鎖定、複製、旗幟標記、探險家地圖)、
織布機與盾牌、結構生物的裝備與嬰兒機率、屬性同步。伺服器 `kiln` release(`KILN_GENERATOR=noise`、種子 60、`KILN_DATAPACK=~/kiln/work/generated`),
世界 `~/wp60/world`,port 25585。**指令只用在「佈置」**(`fill`/`setblock` 蓋東西、`give`、`summon`、`tick rate`、`time set`、`tp`),
實際操作(開 GUI、放物品進漏斗、合成、交易、騎馬、拴繩、點旗幟、取蜂蜜、點傳送門)都走客戶端。

## 環境與工具(相對 wp59 的變動)

| 項目 | 做法 |
| --- | --- |
| 驅動 | `tools/wp58/ClientCtl.java` 新增:`attrlist`(所有屬性與它的 `isClientSyncable`)、`attrs [EID]`(客戶端看到的屬性值)、`entdata EID`(客戶端持有的非預設實體資料、拴繩、縮放)、`mapinfo`(手上地圖的 id/比例/鎖定/顏色雜湊/標記)、`menubutton I`(織布機圖樣等容器按鈕)、`slotitem N`、`comps N`/`comps inv N`(物品的元件) |
| 啟動 | `tools/wp58/wp60_start.sh`(console fifo `~/wp60/console`)、`vm_client60.sh`(`KILN_CLIENT_PORT`、`KILN_CLIENT_NAME` 可改)、`build_agent_wp60.sh`;輔助函式 `tools/wp58/p60.py`(在 `p59.py` 上加了 `vsv`:對 vanilla 參考伺服器下指令) |
| vanilla 參考伺服器 | `tools/wp58/wp60_vanilla.sh`:VM 上跑 vanilla 26.3(種子 60、port 25591、`pause-when-empty-seconds=-1`),用來並排比較:同一個種子的結構位置相同,所以可以直接比較結構生物、馬的繁殖統計、樹的蜂巢 |
| 第二個 Kiln | `~/wp60/s2`(port 25586、全新的世界)用來排除「這個世界的狀態」造成的差異 |
| 場地 | y=200 的 115×109 空中平台(石頭+草方塊),`tick rate 100` 用來等待 |

**遊玩中踩到的坑(不是 Kiln 的問題,但花了很多時間)**:我的玩家在一次高空傳送後死了(`Health 0`),之後的 `tp`/生物生成/死亡畫面都停在「死人」狀態;
死掉的玩家不算在附近玩家,所有非持久生物都不會出現、客戶端的實體清單是空的——一度讓我以為「海底神殿的元素守衛者不見了」。每個腳本開頭要先
`alive()`/檢查 `DeathScreen`。

## 流程與結果

### 1. 比較器鎖定的物品分類器(漏斗鏈供料、三種物品、溢流保護)

我自己設計的(經典的「5 格過濾漏斗 + 比較器鎖定」):
- 主線:一排朝東的漏斗(供料箱 → 漏斗 → …),末端是溢流箱。
- 每種物品一個單元:過濾漏斗 F 在主線漏斗正下方(朝北,不碰任何容器所以自己不會推出去),裡面第 1 格放 1 個目標物品、第 2~5 格各放 1 把木劍(不可堆疊的「雜物」,
  所以 F 只接受目標物品);F 下方再一個漏斗 D(朝北進箱子),D 平時被比較器鎖住。
- 鎖:比較器 c1 讀 F 的訊號(基準 12;目標物品累積到 19 個訊號升到 13),經紅石線(兩個高度差)送進比較器 c2 的側邊;c2 的後方是強度 10 的紅石線,
  所以訊號 ≤ 12 時 c2 輸出 10 鎖住 D,≥ 13 才放行,D 每次抽 1 個。結果過濾漏斗永遠留著 1 個目標 + 4 把木劍 + 17 個緩衝。
- **真實操作**:用客戶端開漏斗 GUI 放過濾物品與木劍(第一版我把木劍放在第 1~4 格,D 先抽走了第 1 格的木劍——所以目標物品一定要放第 1 格);
  用客戶端開供料箱 GUI,shift 點背包把 128 圓石、128 鐵錠、128 橡木板、100 泥土、64 沙、30 煤炭全部倒進去。
- 結果(`tick rate 100`):三個箱子各收到 109 個,過濾漏斗各留 18 個、D 各留 1 個(128 = 109+18+1,守恆),泥土/沙/煤炭全部進溢流箱,**沒有雜物漏進箱子**。
- **溢流保護**:把圓石箱塞滿(27 格 64 個)後再送 512 個圓石:D 吃到 320、過濾漏斗吃滿 64,多出來的 147 個和 64 個泥土進溢流箱,主線沒有堵住。
- 觀察到的 vanilla 行為(Kiln 一致):用 `/setblock` 放的比較器/中繼器不會自己排程第一次更新(要等鄰居更新),玩家放的才會;比較器 `facing` 是**輸入**方向。
- 截圖 `docs/wp60/sorter_overview.jpg`。

### 2. 流浪商人與商人羊駝

- `tick rate 2000` 等了約 60 秒,自然生成一個流浪商人 + 兩隻商人羊駝(`entdata`:羊駝 `leash=wandering_trader`)。
- 交易(MerchantScreen,真的點):買了 12 朵蔥花(交易上限 12 用完 → 缺貨 oos)、4 次橡木原木(32 個),經驗等級 5,進度「What a Deal!」;提示:`selecttrade` 會把整疊綠寶石搬進付款格,關畫面才退回背包。
- 夜晚(`time set 13000`):商人喝隱形藥水(共享旗標 invisible、效果顏色資料);白天恢復。
- 消失:`DespawnDelay` 從 48000 倒數,`tick rate 100` 等到歸零,商人和兩隻羊駝**同時**消失。
- 截圖 `docs/wp60/trader_gui.jpg`。

### 3. 蜜蜂

- 蜜蜂巢 `data merge` 放 3 隻蜜蜂 → 白天離巢、授粉(`HasNectar` 資料旗標)、回巢,`honey_level` 一次一次升(+1)。
- **蜂蜜採集**:`honey_level=5` 的蜂巢,不點營火 → 玻璃瓶取蜂蜜,蜜蜂**生氣**:追著我螫,中毒 10 秒(200 tick,普通難度,血量 20→12;vanilla 的 Bee 在 NORMAL 是 10 秒、HARD 18 秒、EASY 不中毒),
  蜂巢 `honey_level` 歸零;用打火石**真的點燃**營火(蜂巢下方 1 格),再用玻璃瓶/剪刀取蜂蜜與蜂巢蜜(3 個),蜜蜂留在巢裡不生氣。截圖 `bee_campfire.jpg`。
- **樹苗長出的蜂巢**:56 株橡木樹苗(旁邊各放蒲公英)用 `random_tick_speed 3000` 長大,3 棵有蜂巢(約 5%,與 vanilla 的 `oak_bees_005` 相符;並用 vanilla 伺服器在相同佈置下對照:也是 y=101、y=102 各有蜂巢,所以蜂巢出現在樹底的高度是 vanilla 的行為)→ 找到**問題 3**。

### 4. 馬、驢、騾、羊駝

- **屬性同步**:坐在客戶端看馬的 `attrs`:伺服器上這匹馬 `movement_speed=0.2678`、`jump_strength=0.6116`、`max_health=26`,客戶端卻是 vanilla 的預設 0.225 / 0.7 / 53 → **問題 2**(修正後三項一致;騎上去 6 秒,客戶端位置與伺服器位置差 0.2 格,沒有任何 rubber-band)。
- **馴服**:空手對野馬反覆右鍵上馬,每次 `Temper` +5(5、10、15、20、25),第 17 次騎上去就馴服了(vanilla 的規則:`random(maxTemper=100) < temper`)。
- **鞍與馬鎧**:蹲下右鍵開 `HorseInventoryScreen`,把鞍和鐵馬鎧用 GUI 放進去(`equipment` 資料裡 `saddle`、`body`;客戶端 `armor`=5 同步)。截圖 `horse_inventory.jpg`。
- **拴繩**:對馬右鍵(拿拴繩)→ `leash=player`;走遠超過 10 格(我用 `tp` 15 格)繩子斷掉掉出 `lead`。(拴在柵欄上:瞄準柵欄沒問題(命中 `oak_fence`),但後面的測試馬沒拴上,沒有驗證到繩結實體。)
- **繁殖統計(對 vanilla 並排)**:兩匹馴服的馬(速度 0.2/0.3、跳躍 0.5/0.8、血量 20/30,**健康要先補滿**,否則兩邊都不繁殖),各繁殖 12 次:
  vanilla 小馬速度 0.227~0.316(平均 0.263)、跳躍 0.608~0.756(0.677)、血量 19.2~29.4(24.3);Kiln 速度 0.212~0.301(0.251)、跳躍 0.423~0.747(0.629)、血量 20.9~28.0(23.8)——
  範圍與平均吻合;毛色:兩邊的 `Variant` 都有 0、512、515、1027/771 這類組合(父母色、隨機色、隨機斑紋)。→ 找到**問題 5**(修正前 Kiln 的 12 隻小馬完全一樣)與**問題 4**。
- 驢×驢、馬×驢(騾)、羊駝×羊駝:我的對照腳本在 vanilla 這邊只得到零星的資料(vanilla 要滿血、同一區塊等條件,時間不夠),所以沒有做完整的統計比較(見「還沒做到」)。

### 5. 地圖

- **空地圖**:用工作台 GUI 合成(8 紙 + 羅盤),右鍵空氣 → `filled_map`,客戶端 `mapinfo`:id 0、比例 0、中心 (0,0)、`player` 標記 (-30,-60)(= 位置 ×2)。
- **製圖台**:放地圖+紙 → 放大(比例 1,中心變成 64 —— vanilla 的 `createFresh` 把中心重新對齊 256 格網格,所以標記位置是 (-79,-94) 不是 (-15,-30))、
  放地圖+玻璃片 → 鎖定(新 map id、`locked=true`)、鎖定的地圖+空地圖 → 複製(得到 2 張)。截圖 `cartography_zoom.jpg`、`cartography_lock.jpg`。
- **旗幟標記**:對旗幟右鍵 → `banner_white`、`banner_red:Base Camp`(自訂名稱顯示在地圖上),再點一次移除標記。截圖 `map_banners.jpg`。
- **製圖師村民的探險家地圖**:各等級的交易表(1:紙→綠寶石、綠寶石→空地圖;2:稀樹草原/針葉林村莊地圖+窗戶玻璃片;3:試煉空間、海底神殿地圖;4:旗幟、物品展示框;5:林地府邸地圖、地球旗幟圖樣);
  26.3 這些是獨立的物品(`ocean_monument_map` …)。買到的海底神殿地圖右鍵 → 約 30 秒內解析出位置(`/locate` 的最近一座已被「已產生」排除,所以給的是另一座,與 vanilla 的 `skipKnownStructures=true` 一致),標記是 `monument`。

### 6. 織布機與盾牌

- 織布機 GUI:白旗+黑染料 → 圖樣按鈕 0~3 依序是 `square_bottom_left/right`、`square_top_left/right`(vanilla 的 `no_item_required` 標籤順序);
  放苦力怕旗幟圖樣道具 → 圖樣 `creeper`;結果旗幟有 `banner_patterns` 元件。
- 盾牌 + 有圖樣的旗幟 → 工作台合成裝飾盾牌:`base_color=white`、`banner_patterns=[creeper black]`。截圖 `shield_held.jpg`。

### 7. 結構生物的裝備與嬰兒機率(對 vanilla 並排)

vanilla 參考伺服器(種子 60)與 Kiln 在同一批座標,用 `execute … run data get entity` 逐隻比較:

| 結構 | vanilla | Kiln 修正前 | Kiln 修正後 |
| --- | --- | --- | --- |
| 林地府邸 (-1824,1904) | 12 衛道士(鐵斧)、4 喚魔者 | 同 | 同 |
| 海底神殿 (-352,160) | 3 元素守衛者 | 3 | 3 |
| 沼澤小屋 (3664,2688) | 1 女巫、1 貓 | 同 | 同 |
| 村莊 (-384,-544) | 15 村民(1 嬰兒)、1 鐵傀儡、1 貓 | 同 | 同 |
| 堡壘遺跡(下界 (-208,-336)) | 6 豬布林蠻兵**都拿金斧**;17 豬布林(有些穿金頭盔/護腿/靴) | 蠻兵**空手**;豬布林沒有任何金護甲 | 蠻兵都拿金斧;豬布林有金護甲 |
| 海底遺跡溺屍 | 22 隻中 2 把三叉戟、1 把釣竿、1 隻嬰兒 | 沒有裝備、沒有嬰兒 | 修正後依機率(單元測試 4000 隻:約 10% 持三叉戟/釣竿、約 5% 嬰兒) |

→ 找到**問題 6**。(其他的差別來自 Kiln 這邊有玩家在旁邊所以有自然生成的殭屍、豬布林等;vanilla 伺服器沒有玩家。)

### 8. 其他

- **終界出口傳送門與片尾名單**(wp58 留下的):用全新玩家(`Credit2`/`Credit3`)跌進出口傳送門:客戶端確實打開 `WinScreen`(= WIN_GAME 事件 1.0,wp58 的修正有效),
  但在軟體 OpenGL 的客戶端上畫面幾乎立刻自己結束(客戶端用實際經過的時間捲動,每一幀很久)並送出重生請求,所以看不到片尾名單的內容;伺服器收到重生請求後把玩家送回出生點、保持 `seenCredits=1`。這部分不當成 Kiln 問題。
- **客戶端日誌**:整個過程沒有 WARN/ERROR/解碼錯誤(只有 Realms/認證/旁白/空聲音事件這些與伺服器無關的行)。
- **伺服器日誌**:只有我自己 `tp` 到高空自由落下時的 `moved too quickly`(沒有 vanilla 向量,不當成問題)。

## 發現的問題與修正

### 問題 1:只同步了 12 個屬性,vanilla 的 `isClientSyncable` 有 30 個(已修正,有測試)

- 現象:`attribute @p minecraft:scale/gravity/step_height/jump_strength/sneaking_speed/block_interaction_range/entity_interaction_range/movement_efficiency/fall_damage_multiplier/burning_time/…`
  設了之後客戶端的值不變(我用 `attrs` 逐一對了 31 個可同步屬性:19 個沒同步)。客戶端用這些屬性預測移動(重力、跳躍、台階高度、潛行速度)、判斷觸及距離、畫別的玩家的大小。
- 原因:`kiln-sim/src/combat.rs` 的 `EFFECT_SYNCED`/`EQUIPMENT_SYNCED` 是兩份寫死的清單(wp59 補了裝備的 6 個);玩家被別人看到時(`spawn_packets`)完全不送屬性;切換遊戲模式不標記屬性變動。
- 修正:`kiln_entity::mob::attributes::syncable_name`(不能同步的 8 個:擊退抗性/攻擊擊退/攻擊傷害/追蹤範圍/誘餌範圍/增援機率/兩個路標範圍);
  玩家的 `attributes_packet` 送出 `PLAYER_ATTRIBUTES` 裡所有可同步的(30 個)、玩家被配對時也送、遊戲模式改變標記;順便讓 `Player::block_interaction_range` 讀屬性(原本寫死 4.5/5),
  並補上創造模式的 `creative_mode_block_range`(+0.5,vanilla 的 `ServerPlayer.CREATIVE_BLOCK_INTERACTION_RANGE_MODIFIER`)。
- 測試:`kiln-sim/tests/combat.rs::every_syncable_attribute_reaches_the_client`(10 個屬性設值後都在封包裡、伺服器專用屬性不在、切到創造模式送出兩個觸及修飾符)。
  `crowd_golden` 的封包雜湊因此重新錄製(登入/配對的屬性清單變長)。

### 問題 2:生物的屬性從來沒有同步給客戶端(已修正,有測試)

- 現象:客戶端裡的馬 `movement_speed`/`jump_strength`/`max_health` 是 vanilla 預設(0.225/0.7/53),伺服器上是這匹馬自己的(0.2678/0.6116/26);
  騎乘的預測速度、跳躍、血量條都用錯的值;`/attribute` 改僵屍的 `scale` 客戶端也看不到。
- 原因:`kiln-sim/src/entities.rs::spawn_packets` 對生物不送 `ClientboundUpdateAttributesPacket`(vanilla 的 `ServerEntity.sendPairingData` 會送所有可同步的),
  之後的變動(藥水效果、嬰兒速度修飾、馴服、`/attribute`)也沒有任何「髒」偵測。
- 修正:`mobs::attributes_packet`/`attributes_hash`:配對時送;每個 tick 比對可同步屬性的雜湊(基底+修飾符),變了就送(沿用 `meta_sent` 的作法,第一次不重送)。
- 測試:`kiln-sim/tests/combat.rs::a_mobs_attributes_are_sent_when_it_appears_and_when_they_change`。實機:馬的 `attrs` 與 `attribute … get` 一致,騎乘位置沒有 rubber-band。

### 問題 3:樹苗長出來的蜂巢裡沒有活的蜜蜂(已修正,有測試)

- 現象:長出來的蜂巢 `data get block` 顯示有 3 隻蜜蜂,但 `ticks_in_hive` 一直不動(6000 tick 後還是 283/36/374),蜜蜂永遠不出來。
- 原因:樹的 feature 先放下蜂巢方塊(生出沒有蜜蜂的活 block entity),之後才把蜜蜂資料填進**區塊儲存的那份**(`RegionLevel::set_block_entity_data` 只改區塊的 block entity);活的 `Hive` 是空的,
  所以不 tick;存檔時還可能把蜜蜂蓋掉。
- 修正:`set_block_entity_data` 對活的 `BeKind::Beehive` 容器同步重載 `Hive`。
- 測試:`kiln-sim/tests/trees.rs::a_grown_trees_bee_nest_has_living_bees`(長樹直到出現蜂巢,蜜蜂 2~3 隻,30 tick 後每隻 `ticks_in_hive` 都 +30)。

### 問題 4:`/attribute` 把基底設成「註冊表預設值」的屬性不會存檔(已修正,有測試)

- 現象:馬的 `max_health` 設成 20(註冊表預設值,馬自己的是 53)後 `/data merge` 或區塊重載,又變回 53;所以我設了血量的馬一直 `health < max_health` 不能繁殖。
- 原因:`kiln-entity/src/mob/persist.rs` 只存「基底不等於註冊表預設」的屬性。
- 修正:也存「基底不等於這個生物種類預設」的屬性。
- 測試:`kiln-sim/tests/entity_persist.rs::an_attribute_set_to_the_registry_default_is_saved`。

### 問題 5:`/data merge entity` 把生物的隨機數重設,同一對親代的小馬全都一樣(已修正,有測試)

- 現象:對同一對馬重複設 `InLove` 繁殖 12 次,12 隻小馬速度/跳躍/血量/毛色完全一樣。
- 原因:`load_entity_data`(`/data merge`)重新載入生物時用 UUID 重新設定它自己的隨機數;vanilla 的 `Entity.load` 不動 `random`。
- 修正:重載時保留舊的 `random`。
- 測試:`entity_persist.rs::data_merge_keeps_the_entitys_random_so_foals_differ`(4 次繁殖至少 3 種不同速度)。

### 問題 6:結構生成的生物沒有 `finalizeSpawn`(裝備、嬰兒、騎乘)(已修正,有測試)

- 現象:堡壘遺跡的豬布林蠻兵空手(vanilla 都拿金斧)、豬布林沒有金護甲、海底遺跡的溺屍沒有三叉戟/釣竿也沒有嬰兒。
- 原因:`StructureTemplate.placeEntities`(`finalize_entities` 設定)與資料標記生成的生物,Kiln 只用 `finalize_spawn_draws` 消耗亂數,沒有做 `finalizeSpawn`。
- 修正:世界生成在這些生物上放 `kiln:finalize` 欄位(`kiln_world::chunk::FINALIZE_ON_LOAD`;拼圖(jigsaw)結構的模板實體、海底遺跡的溺屍);區塊載入進模擬時
  (`Dim::add_saved_entities`)拿掉欄位並對生物執行 `kiln_entity::mob::finalize_spawn`(`GroupData.structure = true`,用生物自己的隨機數);`Piglin.finalizeSpawn` 在結構來源時不給嬰兒/武器(vanilla 就是這樣);
  `Drowned` 的殭屍鸚鵡螺騎乘也接受結構來源(並設為持久)。
- 測試:`kiln-sim/src/mobs.rs::tests::structure_mobs_are_finalized`(4000 隻溺屍約 10% 持三叉戟/釣竿、約 5% 嬰兒;蠻兵都拿金斧;2000 隻豬布林每個護甲格約 10%、沒有武器也沒有嬰兒)。

## 不是問題但容易誤判的事

- 玩家死了(`Health 0`、`DeathScreen`):附近沒有活的玩家,所有非持久生物都不生成,指令選擇器抓不到東西。每個腳本先檢查。
- 馬要**滿血**(`health >= max_health`)而且已馴服才能繁殖;`/summon` 帶 NBT 的馬不會 `finalizeSpawn`(屬性是預設的 53 血),不帶 NBT 才會隨機;`/data merge entity` 裡的 `Health` 不能超過上限。
- `/attribute` 與 `/data get entity` 只接受單一實體(選擇器要 `limit=1`),兩邊(vanilla 與 Kiln)一致。
- vanilla 伺服器 26.x 沒有玩家時 60 秒後暫停 tick(`pause-when-empty-seconds`),並排比較時要設 `-1`。
- 流浪商人的商品、地圖交易的結果是獨立的物品(`ocean_monument_map` 等),不是帶元件的 `filled_map`。
- 比較器/中繼器 `facing` 是輸入側;用 `/setblock` 放的不會自己啟動第一次更新。

## 最終驗證(VM,分支 wp60-playthrough3,提交 `9ba1a80` 之後只有文件變動)

- `cargo test --workspace --release --no-fail-fast` 不帶 `KILN_DATAPACK`:全綠(EXIT 0)。第一輪(改動後立刻跑)只有 `kiln-sim/tests/crowd_golden.rs` 的 4 個情境失敗——
  狀態雜湊沒變、封包串雜湊變了(登入與配對的 Update Attributes 變長),與 wp45/wp54/wp59 一樣屬於預期的重新錄製;新值 `0x315277188ac2469e`、`0x475d93024d7942ac`、`0xd51533e91aa0dd81`、`0x225295d733920c1d`,檔頭註記 wp60。
- `KILN_DATAPACK=/home/test/kiln/work/generated cargo test --workspace --release --no-fail-fast`:全綠(EXIT 0)。
- `kiln-sim --test determinism`:通過。
- `kiln-entity --test mob_parity`(`work/m6-mobs2/vectors.jsonl`,先備份成 `~/wp60/vectors.jsonl.bak`,沒有新增向量):**1437 個情境全部與 vanilla 一致**(568011 個 mob 狀態)。
- `tools/parity_suites.py --skip-workspace`:102 個 suite 行,全部 exit 0。
- 負向檢查(在 VM 的工作樹上逐項把修正拿掉,各自的新測試都會失敗,之後 `git checkout` 還原):
  生物的屬性不送(`a_mobs_attributes_are_sent_when_it_appears_and_when_they_change`)、玩家只送舊的 12 個(`every_syncable_attribute_reaches_the_client`)、
  蜂巢不載入蜜蜂(`a_grown_trees_bee_nest_has_living_bees`)、屬性基底不存(`an_attribute_set_to_the_registry_default_is_saved`)、
  `/data merge` 重設隨機數(`data_merge_keeps_the_entitys_random_so_foals_differ`)、豬布林在結構裡仍給武器/嬰兒(`structure_mobs_are_finalized`)。
- VM 上 `~/wt/wp60-playthrough3/target` 已刪;客戶端、伺服器(Kiln、第二個 Kiln、vanilla)已停。
## 還沒做到 / 後續

- 驢×驢、騾、羊駝的繁殖與強度遺傳:Kiln 與 vanilla 的統計並排沒有完成(`Equine::offspring_attributes` 與馬共用,馬的統計吻合)。
- 拴在柵欄上的繩結實體、被繩子牽著的馬跟著玩家走。
- 殭屍村民治療後的交易折扣數值、村民閒聊(gossip)。
- 片尾名單的內容(軟體 OpenGL 客戶端上畫面幾乎立刻結束)。
- 地圖顏色與 vanilla 並排(我只驗證了流程、標記、縮放/鎖定/複製與探險家地圖;地圖的像素顏色靠既有的單元測試)。
- 蜜蜂授粉對作物的催熟、蜜蜂對花的搜尋範圍。
- 結構生物的「亂數序列」與 vanilla 不同(vanilla 用世界生成的亂數,Kiln 用生物自己的),只保證分佈一致,不保證哪隻生物拿什麼。
