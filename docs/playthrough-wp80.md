# wp80:第十次遊玩(海底神殿、終界閘道、林地府邸與前哨站、悅靈、考古與嗅探獸、蒼白之園、信標與銅、羅盤與激流;每個玩家受到的傷害封包)

分支 `wp80-playthrough10`(從 main `302681f4` 開出)。方法沿用 wp72~wp79:**比較的兩邊必須是同一個世界**(種子 80、noise 產生器、視距與模擬距離 8)、
樣本不夠不下結論、鎖步(`tick freeze`+`tick step`/`tick sprint`)能得到位元精確的就不用統計、vanilla 自己不決定性的(跨區塊特徵)不當依據。
所有建置與測試只在 VM(`~/kt wp80-playthrough10 …`,`CARGO_BUILD_JOBS=3~4`);Kiln 25589、vanilla 25595。工具在 `tools/wp80/`(`push_tools.sh` 複製到 VM 的 `~/wp80`)。

這一輪最有效的新方法是**封包畫廊**:同一件事在兩個真客戶端上各做一次,把客戶端收到的封包種類、音效、粒子取集合比較(`p80_gallery.py` 44 種怪物各對生存玩家 12 秒、`p80_proj.py` 各種投射物、`p80_matrix.py` 物品×生物)。
它在不知道要找什麼的情況下找到最多問題(玩家被投射物打不會晃鏡頭與擊退、生物落地沒有塵土粒子、羅盤對磁石無反應、閘道不傳送生物與物品)。

## 工具(`tools/wp80/`)

| 項目 | 做法 |
|---|---|
| `lock.py`、`lab80.py` | wp78 的鎖步工具 + 實驗室輔助:`tank`/`box`、`nbt()`(兩邊 `data get` 去掉前綴)、`count_blocks()`(用 `fill … replace` 的數量計方塊)、`cmpn()` |
| `par.sh`、`switch.sh`、`actor.sh`、`wp80_*.sh`、`agent/` | 從 wp78 複製、改名 wp80(Kiln 預設種子 80、vanilla `VSEED` 80) |
| `p80_guard.py`、`p80_elder.py` | 守衛者雷射(水槽內的生存玩家,每 5 刻取樣生命)、長老守衛者的挖掘疲勞(效果的出現時間、持續時間、重新施加,客戶端的 `game_event 10`) |
| `p80_monument.py`、`p80_conduit.py` | 海綿(吸水、植物、半磚、地獄烘乾)與導管(框架 15/16/29/41/42、能量、範圍、對敵對生物的傷害) |
| `p80_hurt.py`、`p80_proj.py`、`p80_gallery.py`、`p80_one.py`、`p80_lines.py`、`p80_fall.py` | **封包畫廊**:44 種怪物在封閉場地(障礙方塊牆)對生存玩家 12 秒,比較兩邊客戶端收到的封包種類/音效/粒子(只有 K 有、只有 V 有的),再用 `p80_lines.py` 看單一封包;`p80_proj.py` 各種投射物打玩家;`p80_fall.py` 玩家落地 |
| `p80_struct.py` | 兩邊 `locate` 結構是否同座標,載入後的生物普查 |
| `p80_allay.py` | 悅靈跳舞(唱片機)與紫水晶複製(真點擊) |
| `p80_pale.py` | 眼眨花:1600 朵,日夜與黃昏/黎明邊界 |
| `p80_bonemeal.py` | 骨粉(用發射器),每種植物 16~40 個格子的方塊數統計 |
| `p80_gateway.py`、`p80_crops.py`、`p80_sniffer.py`、`p80_copper.py`、`p80_beacon.py`、`p80_beaudit.py` | 閘道、作物生長、嗅探獸蛋與挖掘、銅燈泡與閃電、信標、方塊實體 `data get block` 稽核(21 種) |
| `p80_small8.py`、`p80_omen.py`、`p80_matrix.py`(+`p80_matrix_cmp.py`)、`p80_shots.py` | 羅盤與激流、不祥之兆→襲擊、物品×生物矩陣、截圖 |
| `neg80.sh` | 負向檢查:把修正過的檔案退回修正前,對應的測試必須失敗 |

## 踩到的坑(不是 Kiln 的問題)

1. **`~/wp80/agent` 被 `build_agent_wp80.sh` 的 `rm -rf` 清掉**(同一個目錄放 vanilla agent 原始碼與客戶端 agent 的輸出):改輸出到 `clientagent/`。
2. **重啟 Kiln 時預設種子是 72**(`wp80_kiln_keep.sh` 的 `${KILN_SEED:-72}` 從 wp78 複製過來):結構位置兩邊不同(`locate` 全部 DIFF),查到才砍世界重來。
3. **建置時 vanilla 伺服器被 OOM killer 殺掉**(VM 只有 10 GB、另一個 session 也在跑):之後的「V」數字讀的是死伺服器的舊 log(全 0)。建置前先停 vanilla 與客戶端。
4. `fill` 超過 32768 格靜默失敗(導管水池、作物排)。
5. **`instant_health` 振幅 30**:`4 << 30` 在 Java 是 0,什麼都不補;用 10。
6. vanilla 的光照引擎在 `fill` 後不是立刻更新:同一刻放的作物(匹克植物)會被「光線不足」拆掉(Kiln 的光照同步更新)——測試場地要先填、等一下、再放作物。
7. 平台腳本(`platform(0,0,40,40,y=199)`)留下的石板擋住信標的光束:信標場地要先清掉。
8. 沒有玩家時 Kiln 的主控台指令與發射器 tick 很慢(整個腳本 7 分鐘沒有輸出):要連著客戶端。
9. `tick freeze` 之後玩家的效果持續時間仍照真實時間減少(玩家 tick 不被凍結),所以凍結下比效果的剩餘時間沒有意義;只比「哪一步出現」。
10. 資料格式:26.x 的信標欄位是 `primary_effect`/`secondary_effect`、閘道的出口是 `exit_portal:[I;x,y,z]`;舊的 `Primary`/`ExitPortal:{X,Y,Z}` 兩邊都忽略。
11. `pgrep -f`/`pkill -f` 的樣式出現在自己的 ssh 指令列會殺掉自己或永遠不結束(`[.]`、`releas[e]` 的括號寫法沒有用,因為指令列裡的原文也會被比對到):改用 PID。

## 玩了什麼、結果(K = Kiln、V = vanilla)

### 1. 海底神殿
* **守衛者雷射**(兩個真玩家各在 13×13 的水槽裡,守衛者在 6 格外,3 輪 × 500 刻):每擊 **6.0**,第一擊在 90~100 刻(80 刻鎖定 + 目標選擇),間隔約 95~100 刻;K 與 V 的擊數 14/17(大雜訊內)。偶爾 5 刻內連兩擊兩邊都有。
* **長老守衛者**:挖掘疲勞 III(持續 6000、放大 2),第一次出現在 0~1200 刻內(取決於實體 id),客戶端收到 `game_event 10`(詛咒動畫)與音效;每 100 刻取樣的持續時間遞減曲線相同。
* **海綿**:9×9 水池放海綿:水量 660→675 兩邊相同;植物(海帶、海草、半磚、高海草)的吸收、再放兩個海綿、濕海綿在地獄烘乾成海綿:**全部相同**。
* **導管**:框架 15/16/29/41/42 個:15 不啟動、16 起給能量、42 才攻擊;敵對殭屍(NoAI,水中 5 格)每 40 刻 20→16→12→4,20 格外與牛不受傷:**逐項相同**(`p80_conduit.py`)。

### 2. 終界
* **終界閘道**:`exit_portal`(精確)與非精確(找最高方塊)、沒有出口(找外島,`(950.5, 66, 425.5)`):豬與物品的落點兩邊逐位相同;**問題 5**:修好前 Kiln 只傳送玩家,生物與物品留在閘道裡。
* **潛影貝**:`hitByShulkerBullet`(開殼被子彈打到:傳送並可能留下複製)、`teleportSomewhere` 與 Kiln 的 Rust 逐行對 javap(沒有發現差異);用指令生成的子彈在兩邊都不飛(Target 不認),所以沒有統計比較。終界城與船、鞘翅、箱子戰利品 wp65 已做。

### 3. 林地府邸與前哨站、悅靈
* `locate`:前哨站 `(-672,~,-2368)`、府邸 `(3072,~,480)` 兩邊相同;載入後普查:**府邸 衛道士 20、喚魔者 4(K = V)**,前哨站:悅靈 2、鐵傀儡 2、運輸礦車(箱子)2(物品 49 對 47,地上物品,雜訊)。
* **悅靈**(唱片機 + 紫水晶,真點擊,凍結讓悅靈不動):先放悅靈再放唱片 → 兩邊都跳舞(實體資料 16=true)→ 紫水晶複製出第二隻,冷卻 6000,`entity_event 18`:**相同**。
* **不祥之兆**(真玩家在村莊 (−144,~,640)):`bad_omen` 放大 1 → 2 s 內變成 `raid_omen`、29~30 s 出現 Raid 血條、15 s 填滿,兩邊相同;**之後**:K 在血條滿的當刻 5 名襲擊者同時出現,V 約 17 s 後才一個個出現(1→2→4→5)。沒有查原因(襲擊的生成位置與等待在 wp66/wp78 已並排比過,這一輪只有一次取樣)。

### 4. 考古、嗅探獸、作物
* **作物**(`p80_crops.py`,800 株、400 刻、隨機刻速度 100):火炬花、匹克植物、小麥、甜菜、甜漿果叢、地獄疣各階段株數兩邊在抽樣雜訊內(例:匹克植物 K 195/272/205/84/44、V 184/288/193/98/37)。
* **嗅探獸蛋**(`sprint 14000`,單顆):苔蘚上第一次裂 ≈ 4000 刻、第二次 ≈ 8000、孵化 12000~13000;沙上第一次裂 ≈ 8000~9000:**兩邊逐階段相同**。**嗅探獸挖掘**(10 隻、6000 刻):火炬花種子 6、匹克植物種莢 4(兩邊相同)。
* **骨粉**:見問題 4(苔蘚、竹子、滴水葉、海草、…)。

### 5. 快樂恐魔、騎乘與長矛
**沒有重做**(wp68、wp74 做過孵化、雪球、成長、掛具、四人同騎;wp68 做過鸚鵡螺、駱駝屍殼、長矛、蒼白之園的嘎吱怪)。這一輪只用封包畫廊對 `happy_ghast`/`camel`/`horse`/`zombie_horse`/`skeleton_horse`/`nautilus` 做了**物品互動矩陣**(見 8)。

### 6. 蒼白之園
* **眼眨花**(`p80_pale.py`,1600 朵、隨機刻 4096):白天全閉、夜晚全開;黃昏 12500 前閉、後開;黎明 23500 前開、後閉:**修好後兩邊逐點相同**(問題 3;修好前 Kiln 的眼眨花從不改變)。
* **蒼白苔蘚骨粉**:問題 4。**嘎吱怪**:封包畫廊 `creaking`(12 s)兩邊只有 `entity.creaking.activate/freeze` 的取樣差。

### 7. 信標、銅、羅盤、激流
* **信標**(`p80_beacon.py`、`p80_beaudit.py`):金字塔 1~4 層、混金屬、第 4 層缺一格(2 層)、光束上有石頭(0 層)、四種金屬:`Levels` 兩邊相同;**問題 6**(`data get block` 顯示 0)與**問題 7**(`data merge` 後重算)。效果與範圍:放大、範圍 50(`levels*10+10`)內外一致。
* **銅燈泡**:5 種(含上蠟)× 4 次脈衝:`lit`/`powered` 逐步相同。**閃電與銅**:閃電清掉氧化(隨機走到鄰居)兩邊都清自己與一個鄰居(方向不同,各自的亂數),上蠟的不動:相同。
* **羅盤**:問題 8。**激流**:水裡衝 17.1 格兩邊相同、乾地/雨中不動(該生物群系不下雨)兩邊相同。
* **方塊實體 `data get block` 稽核**(21 種,凍結後逐欄位比):信標(問題 6)、合成器(問題 6 的同一個原因);其餘:釀造台 `BrewTime` 差 1 刻(K 在放置那一刻就開始倒數)、漏斗 `TransferCooldown` K −1 對 V 0、講台 `Page`、唱片機 `is_playing`(寫進去的欄位 K 原樣回傳,vanilla 忽略):沒有修。

### 8. 物品×生物互動矩陣與封包畫廊
* `p80_gallery.py`:44 種怪物 12 秒:大部分相同;差異:**玩家被箭、火球、三叉戟、風彈、守衛者雷射打不會有 `HurtAnimation`**(問題 1)、**生物落地沒有塵土粒子**(問題 2,破壞者/風靈的「particle block」);不同的只有 `ProjectilePower`(火球加速度,預設 0.1 客戶端預設值相同,沒做)與 `PlayerCombatEnter/End`(沒有客戶端效果,沒做)。
* `p80_matrix.py`(物品×生物,真客戶端 NoAI 生物 2 格外、右鍵):**第一段 176 組**(牛、羊、豬、雞、馬、驢、羊駝、駱駝、貓 × 20 種物品;VM 負載 10+):**餵食的 `LoveCause` 沒有存**(問題 9);其餘差異都是兩邊客戶端點擊落空的雜訊(同一個組合重跑會翻轉:羊的剪刀、拴繩);水桶對生物 K 多送一個 `ContainerSetSlot`(物品欄重同步,沒有影響)。
  **第二段 195 組**(狼、狐狸、貓熊、兔、海龜、美西螈、青蛙、山羊、嗅探獸、犰狳 × 19 種物品):**0 組不同**(物品欄變化、NBT 變動的鍵、客戶端收到的音效/粒子/事件都相同)。其餘的生物(蜜蜂、鸚鵡、悅靈、快樂恐魔、村民、傀儡…)沒有跑完(約 370 組後停掉,負載太高)。

## 找到並修好的問題

| # | 問題 | 修正 | 測試(拿掉修正會紅,`neg80.sh` 已驗) |
|---|---|---|---|
| 1 | **玩家被箭、火球、風彈、三叉戟、守衛者雷射、墜落的鐵砧打到,客戶端沒有收到 `Hurt Animation`(鏡頭晃動),也沒有預設擊退**:`health.rs` 只對近戰與荊棘(`MobAttack`/`Thorns`/`PlayerAttack`/`Other`)做 `dealDefaultKnockback` 與 `indicateDamage`,vanilla 是 `#no_knockback` 以外的所有傷害(來源位置:`getSourcePosition`,沒有位置時隨機方向與跳一下) | `health.rs` 條件改成 `!no_knockback && !(blocked && amount <= 0)`,方向用來源位置(投射物的位置是飛行方向後方一點)否則攻擊者位置,`entities.rs::hurt_player` 把所有 `DamageSource.pos` 傳進來 | `kiln-sim/tests/player_hurt_animation.rs`(箭、小火球、風彈:1 個 `HurtAnimation`;`/damage mob_attack` 無位置:仍有;`generic`、`cactus`:沒有) |
| 2 | **生物與玩家落地沒有塵土粒子**(`LivingEntity.checkFallDamage`:超過安全落差時 `sendParticles(BlockParticleOption(block, 腳下方塊), 150 × min(0.2 + power/15, 2.5) 個, 速度 0.15)`) | `entity.rs` 記下落地,`mob/mod.rs::landing_dust` 發粒子;玩家端 `fall.rs::landing_dust` + `queue_block_particles` | `kiln-sim/tests/landing_dust.rs`(5 格:1 個封包;1 格:沒有);真客戶端:牛 5 格 K n=50、V n=50 |
| 3 | **眼眨花從不開合**:Kiln 沒有 `EyeblossomBlock`(隨機刻、排程刻:跟隨 `gameplay/eyeblossom_open` 屬性,改變時灑 trail 粒子、播長音效,同種類的鄰居(±3,±2,±3 內)依距離排程跟著變並播短音效) | `kiln-blocks/src/behaviour/misc.rs::eyeblossom_*`、`Level::eyeblossom_open`、`Effect::TrailParticle`,`sim/blocks.rs` 送封包 | `kiln-sim/tests/eyeblossom.rs`;鎖步 `p80_pale.py` 10 個時間點逐點相同(1600 朵) |
| 4 | **骨粉對很多方塊沒有作用**:苔蘚/蒼白苔蘚(長出一片)、竹子與竹筍、灌木/螢火蟲灌木/乾草(擴散)、懸垂苔蘚、生根泥土、海草、海泡菜、匹克植物、海帶與各種藤蔓(含洞穴藤蔓漿果)、小/大滴水葉、紅樹葉、花簇(粉紅花瓣)、地獄岩/菌岩/地獄菌;**水下骨粉**(`growWaterPlant`:128 次撒海草,暖海加珊瑚)也沒有 | 新 `kiln-blocks/src/behaviour/bonemeal.rs`(逐行對 javap:亂數順序、`Util.shuffle`、`BlockScanUtils` 的掃描順序)、`tools.rs`/`dispense_items.rs` 接上、`grow_water_plant` | `bonemeal.rs` 單元測試 9 個、`bone_meal_dispenser.rs`;鎖步統計 `p80_bonemeal.py`:苔蘚 K 416.9/V 418.6、竹子 404.4/404.4、小滴水葉 405.4/405.3(修前 403.0) |
| 5 | **閘道只傳送玩家**:生物、物品、載具在 `TheEndGatewayBlock` 裡不動(`entity_portals` 的 `PortalKind::Gateway => None`) | 偵測閘道方塊、用玩家用的 `gateway_destination`,投射物不帶(合併到 main 時改用 wp81 同一功能的實作:同一個實體在同一維度裡移動,投射物也會過,原版的 `AbstractArrow`/`ThrownTrident` 沒有覆寫 `canUsePortal`) | `kiln-sim/tests/gateway_entities.rs`(豬、物品);鎖步 `p80_gateway.py`:精確、非精確、無出口三種落點與 V 逐位相同 |
| 6 | **`data get block` 的信標 `Levels` 永遠是 0、合成器沒有欄位**:容器只在「變動過」時才寫回區塊,信標的 levels 與合成器的預設欄位不會標變動 | `store_one`:信標一律寫;`store_at`(給指令讀的)一律寫 | `beacon_pyramids.rs`(層數、`data get block`、指令放的合成器) |
| 7 | **`data merge block` 設信標的力量後,信標忘掉光束與層數**(`reload` 重新建立),要再等 1~2 個 80 刻脈衝才給效果 | `reload` 保留 `levels`/`last_check_y`/`beam` | `beacon_pyramids.rs::a_data_merge_keeps_the_levels_and_the_beam…` |
| 8 | **羅盤對磁石使用沒有反應**,追蹤羅盤也不會在磁石消失後忘掉目標(`CompassItem.useOn`/`inventoryTick`) | `tools.rs::lodestone_compass`(音效 `item.lodestone_compass.lock`、拆一個、創造模式不減)、`lodestone.rs::tick_compasses` | `kiln-sim/tests/lodestone_compass.rs` |
| 9 | **餵食的 `LoveCause`(玩家 UUID)沒有存**(`data get entity` 沒有、重新載入後繁殖不算玩家) | `MobData.love_cause_uuid`、`persist.rs` 存取、`breed::love_cause` 以 UUID 找回在線玩家 | `kiln-sim/tests/love_cause.rs` |

## 沒有結論或沒修的

| 項目 | 狀況 |
|---|---|
| 骨粉:發光地衣/幽匿脈(多面方塊擴散)、蒼白苔蘚地毯的頂部、棚狀蘑菇 | 沒做(javap 看過,多面擴散器要整套 `MultifaceSpreader`) |
| 突襲:血條滿後 K 一次 5 名襲擊者、V 約 17 s 後陸續出現 | 一次取樣,沒查(襲擊在 wp66/wp78 並排比過) |
| 封包:`ProjectilePower`(火球加速度)、`PlayerCombatEnter/End` | 沒有客戶端可見效果,沒做 |
| 釀造台 `BrewTime` 差 1 刻、漏斗 `TransferCooldown`、講台 `Page`、唱片機 `is_playing` 的 NBT 顯示差異 | 沒修(實際的時序向量都相同) |
| 閃電清銅的隨機走向 | 兩邊各自的亂數,結果的種類相同 |
| 快樂恐魔、鸚鵡螺/殭屍鸚鵡螺、駱駝屍殼騎手、長矛突進、試煉不祥模式、陶罐、刷子、兩位玩家打凋零、監守者潛行、傳送門被打斷 | **沒做**(wp68/wp74/wp78 已涵蓋大部分的基本行為;這一輪的負載太高,真客戶端腳本跑不穩) |
| 物品×生物矩陣只跑了約 370 組(19 種生物) | 剩下的生物(蜜蜂、鸚鵡、悅靈、快樂恐魔、村民、傀儡、哞菇、行者獸、豬獸、殭屍馬…)要在安靜的機器上重跑(`p80_matrix.py` + `p80_matrix_cmp.py`) |

## MSPT 與區塊

只有最後一次啟動(物品×生物矩陣,一位玩家在平台上)的 Kiln log 還在(前面每次重啟都覆寫了 log):`mspt` 報告 64 筆(63 筆有玩家):平均的平均 **0.48 ms**、中位數 0.46 ms、p99 的中位數 2.2 ms,最大平均 0.79 ms,沒有任何一筆單刻 > 400 ms 或平均 > 5 ms;區塊:`disk_hits=379`、`sync_loads=0`、`gen_done=0`(世界已生成)。
遊玩當中沒有看到異常的停頓;兩個舊問題照舊(wp73、wp78 記過):`forceload`/tp 到沒生成的地方、終界閘道出口(950,66,425)的冷生成會讓一個 tick 停 2~3 秒,vanilla 同樣會。這一輪的 VM 負載(另一個 session 的測試)常在 10 以上,所以延遲數字不當依據。

## 截圖

`docs/wp80/eyeblossom_KV.jpg`:夜晚的蒼白苔蘚上,眼眨花全部開啟(兩個客戶端:Kiln 大視窗、vanilla 小視窗疊在上面)。

## 驗證(VM,最後一個程式碼 commit `wp80: pale moss carpet test`;之後只改文件與驗證腳本)

* `cargo test --release --workspace --no-fail-fast`:**不帶**與**帶** `KILN_DATAPACK=/home/test/kiln/work/generated` 各一次(`tools/wp80/verify80.sh`):都是 **1441 通過、0 失敗、23 個本來就 ignore**(wp78 是 1410;新增 31 個:`player_hurt_animation`(2)、`landing_dust`(1)、`eyeblossom`(1)、`bone_meal_dispenser`(1)、`beacon_pyramids`(4)、`gateway_entities`(2)、`lodestone_compass`(1)、`love_cause`(1)、`bonemeal.rs` 單元測試(10)…);含 `determinism.rs`(各 6 個)、`mob_parity`(`mobs_match_vanilla`,`work/m6-mobs2/vectors.jsonl` 全部案例)、`crowd_golden`(4 個)。
* `tools/parity_suites.py --skip-workspace`:**113 個步驟全部 exit 0**(`container parity` 30+14 個 scenario、`sculk` 4160/4160、`spear` 148、`mob_parity`…)。
* `crowd_golden` **沒有重錄**:玩家封包與狀態雜湊都沒變(測試場景裡沒有玩家被投射物打到,也沒有生物落地)。
* 沒有新增向量檔(`work/wp80/` 空);`work/m6-mobs2/vectors.jsonl` 沒有動。證據是鎖步並排與真客戶端並排(本文各節)。
* 負向檢查(`tools/wp80/neg80.sh`,把修正過的檔案或那一行退回,對應的測試必須失敗):玩家受傷封包、落地塵土、眼眨花(排程刻與隨機刻兩處)、骨粉(發射器)、信標 `data get block`、指令放的合成器欄位、`data merge` 保留信標層數、閘道實體、羅盤、`LoveCause`:**11 項全部失敗(good)**。
