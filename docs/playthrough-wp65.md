# wp65:第五次真實客戶端遊玩(兩個真實客戶端同時在線 + vanilla 26.3 參考伺服器並排)

分支 `wp65-playthrough5`(從 main `c174a46a` 開出)。做法同 wp58~wp64:官方 26.3 客戶端跑在 VM 的 Xvfb + llvmpipe 上,由 `tools/wp58/ClientCtl.java`(java agent)驅動
(客戶端不在 Windows 上跑)。**這次同時有兩個真實客戶端**:演員 `KilnTest`(agent 25999)與第二位玩家 `Obs`(agent 26000),兩個都是真的玩家,不只是旁觀者。
伺服器:Kiln(`KILN_GENERATOR=noise`、種子 65、`KILN_DATAPACK=~/kiln/work/generated`,port 25587)與 vanilla 26.3 參考伺服器(同種子、port 25593)。
**同一個種子,兩邊的地形、結構位置、結構箱子的戰利品種子完全一樣**(見「先驗證的事」),所以可以逐項並排比對,不必只靠推論。
指令只用在佈置(平台、`give`、`tp`、`tick rate`、`kill` 清場、`fill` 蓋機器);戰鬥、右鍵、開 GUI、騎乘、按鍵、走進傳送門、喝瓶子、從展示框拿東西都走客戶端。
(Mojang 的資料沒有進 repo;截圖只有 Xvfb 的遊戲畫面,放在 `docs/wp65/`。)

## 工具(相對 wp64 的變動)

| 項目 | 做法 |
| --- | --- |
| 驅動 | `ClientCtl` 新增 `ent EID`:客戶端眼中另一個實體的位置、姿勢、四件護甲、主/副手、使用/格擋、揮手動畫、載具與乘客、睡覺/滑翔/著火/隱形 |
| 兩個客戶端 | `tools/wp58/p65.py`(`clients()`、`ent()`、`go()`、`mspt()`、`chunks()`、`alive_b()`:第二個視窗只有 480×270,重生按鈕的位置不同);`vm_observer65.sh` 可調畫質,第二位玩家是完整玩家 |
| 場景腳本(`TARGET=K|V`) | `p65_b1.py`(PvP、聊天、丟物品、船、睡覺)、`p65_b2.py`(下界傳送門連結)、`p65_spawn.py`(自然生怪統計)、`p65_loc.py`/`p65_cities*.py`(`locate`)、`p65_chest.py`、`p65_end.py`(龍死後的閘門)、`p65_city.py`(終界城的船)、`p65_elytra.py`(鞘翅+煙火火箭,量 mspt 與區塊)、`p65_glide*.py`、`p65_strider.py`、`p65_barter.py`、`p65_ghast.py`、`p65_raid.py`、`p65_farms.py`、`p65_tower.py`、`p65_iron.py`、`p65_look.py`、`p65_horse_e.py` |
| 啟動 | `wp65_start.sh`、`wp65_vanilla.sh`、`vm_client65.sh`、`vm_observer65.sh`、`build_agent_wp65.sh` |

踩到的坑(不是 Kiln 的問題):`pkill -f` 的樣式出現在自己的 ssh 指令列上會殺掉自己的 shell(用 `[p]attern` 寫法);主控台 `/tp` 在主世界執行,在下界要 `execute in minecraft:the_nether run tp`;
站在傳送門裡不離開會讓玩家傳送冷卻一直補滿(vanilla 也是);`import p64` 會把 `VDIR` 指到 wp64;`effect clear` 把 resistance 清掉後摔死,之後的 `tp` 都是對死人;
兩個窗口疊在一起,第二個窗口的死亡畫面會蓋在截圖中間;**斷線的客戶端會把 CPU 吃光(兩個 JVM 共 800%),VM 負載 18,vanilla 伺服器兩次被拖死/消失**(沒有日誌,用 `van2.sh` 在原世界續跑),
之後都先停掉客戶端再重啟;sed 把 `\d` 弄壞了一個計數用的正規式,害我以為農場「兩邊都 0」(見「差異」)。

## 先驗證的事(Kiln 與 vanilla 完全一致)

- **結構與生物群系位置**:12 個 `locate`(堡壘 (272,~,240)、堡壘遺跡 (-192,~,-80)、玄武岩三角洲/靈魂沙谷/扭曲森林/緋紅森林、終界城、村莊、前哨站、要塞…)與 16 座終界城的座標逐項相同。
- **結構箱子的戰利品**:下界要塞 5 個箱子的 `LootTableSeed` 逐位元相同、打開後內容相同;終界城船的兩個箱子(鑽石 1+2+5+1+1、金錠、鐵頭盔、裝飾模板…)相同;船上的展示框是鞘翅;15 隻潛影貝、船邊 3 隻。
- **下界傳送門連結**(兩個真實玩家一起站在同一個門裡,走的是真正的 80 tick 等待):P1 (808,200,808) → 下界 (97.5,63,101.5);離開再進、P2 (888,…) 距離 14 格在 16 格搜尋半徑內 → **連到同一個下界門**;
  P3 (968,…) 距離 24 格 → **新建**下界門在 (129.5,101,112.5);從下界回來:第一個門回到 P1、第二個門回到 P3;第二位玩家晚 0.4~0.9 秒到達。兩個伺服器逐項相同,連**新門的位置**都一樣。
- **終界閘門**:龍死後第一個閘門在 (96,75,0),出口 (971.5,58,32.5),兩邊相同(但有一個差異,見問題 3)。

## 玩了什麼、結果

### 多人互動(`p65_b1.py`、`p65_look.py`,Kiln 與 vanilla 各跑一遍)
- **PvP**:B 用鑽石劍打穿鐵胸甲的 A:6.3(滿蓄力;vanilla 在同樣情況下 4.8~5.47,差在客戶端點擊與伺服器 tick 的相位,不是規則);舉盾:劍 0 傷害;斧:盾被打下(`using/blocking` 兩邊的觀看者都看到 `False`)、冷卻中舉盾看不到格擋、斧打在沒舉盾的 A 上 8.46(vanilla 7.96);盾的冷卻過了以後又擋得住。`docs/wp65/pvp1.jpg`
- **別人眼中的我**:B 的客戶端看到 A 穿四件不同護甲(含皮靴染色)、主手隨快捷欄換、副手盾、揮手動畫的樣本、拉弓 `using`、蹲下姿勢、衝刺、隱形時裝備隱藏——全部與 vanilla 相同。
- **聊天、私訊、`/say`**、**丟物品與撿起**(兩邊都是一次丟一個,B 走過去撿)、**兩人同坐一艘船**(兩個乘客,A 划 3 秒,B 與 A 相距 0.8,B 蹲下下船)、**睡覺百分比**(A 睡著 B 醒著:6 秒後時間照走;`players_sleeping_percentage 50` 後立刻跳過夜晚)——與 vanilla 一致。
- 兩個人同時走進同一個傳送門、同時走進同一個終界閘門(後者找到問題 3)。

### 下界(`p65_n1/n2.py`、`p65_spawn.py`、`p65_strider.py`、`p65_barter.py`、`p65_ghast.py`)
- 真的用鑽石鎬挖出一段隧道、從傳送門走到堡壘(走路 220 格穿過洞穴太慢,後半用指令);在堡壘的烈焰人刷怪籠前兩個玩家一起打烈焰人,掉烈焰棒;`docs/wp65/fortress_fight.jpg`。
- **自然生怪統計**(兩個創造模式玩家站在堡壘與緋紅森林,`tick rate 100`,4 次取樣各 150 遊戲秒,最後用 `kill` 計數)——**找到問題 1 與 2**。修正後(Kiln : vanilla):

  | | 堡壘 K : V | 緋紅森林 K : V |
  | --- | --- | --- |
  | 烈焰人 | 15 : 9 | – |
  | 凋零骷髏 | 10 : 19 | – |
  | 殭屍豬人 | 123 : 129 | 113 : 138 |
  | 豬布林 | 119 : 135 | 110 : 135 |
  | **疣豬獸** | **88 : 100**(修正前 6) | **92 : 70**(修正前 **0**) |
  | **熾足獸** | **49 : 48**(修正前 **0**) | **59 : 41**(修正前 **0**) |
  | 岩漿方塊 | 18 : 6 | 3 : 1 |
  | 幽靈 / 骷髏 / 終界使者 | 0 / 3 / 0 : 2 / 8 / 0 | 2 / 64 / 5 : 2 / 35 / 6 |

  (骷髏與岩漿方塊的差距在雜訊範圍,沒有追;`tick rate` 下 vanilla 單執行緒只跑得到 16~19 tps,Kiln 40~70 tps,所以比的是每個遊戲 tick 的生怪量。)
- 熾足獸:真的給鞍、右鍵裝上、再右鍵騎上、拿扭曲蕈菌釣竿,騎乘位置與速度和 vanilla 相同量級(測試台的熔岩池很窄,速度差只是路徑長度);豬布林交易:A 戴金頭盔、右鍵丟金錠,11 次交易得到火焰彈、末影珍珠、鐵粒、噴濺藥水、礫石、黑曜石、黑石(vanilla 那次只湊到 2 次交易,因為豬布林跑開,分布不比);
  靈魂沙上走 1 秒 2.5 格(下界岩 4.2 格),**與 vanilla 相同**;**幽靈**真的射出火球、爆炸傷害(客戶端看到血量 20 → 18.8)、兩個玩家都被打死過(resistance 清掉後),幽靈會離開 70 格外——行為與 vanilla 相同。

### 終界(`p65_end.py`、`p65_city.py`、`p65_elytra.py`)
- 殺掉龍:出口傳送門、龍蛋在 (0,62,0)、閘門在 (96,75,0);**兩個玩家同時走進閘門,第二個沒被傳走**——問題 3。出口島的終界石、出口閘門在 (972,67,31),與 vanilla 相同。
- 終界城船 (1202,89,1330)(終界城 (1280,1328)):走到船裡,**真的打展示框**——第一下被潛影貝的子彈擋掉、第二下鞘翅進背包、第三下框進背包;裝上鞘翅;被潛影貝子彈打中得到懸浮——與 vanilla 相同。
- **鞘翅+煙火火箭**:啟動滑翔是 `START_FALL_FLYING`——**問題 4**(伺服器端原本完全不認)。修正後:

  | 路線(起點平台,A 真的跳起來再按一次跳躍打開翼) | 速度 | 距離 | 區塊 | mspt 平均(30 秒視窗) | p99 | 最大 |
  | --- | --- | --- | --- | --- | --- | --- |
  | 主世界 x:9000→13449,150 秒、75 支火箭 | 32.8 格/秒 | 4450 格 | 3467 個新區塊(23 個/秒;平均 319 ms/個,p99 1.1 s) | 2.9~4.1 ms | 16~22 ms | 25 ms |
  | 下界頂上 y≈180 x:2000→6528,150 秒、72 支 | 32.8 格/秒 | 4500 格 | 約 3600 個(24 個/秒) | 4.0~5.9 ms | 約 30 ms | 49 ms |

  (開頭那次 3.4~5.2 秒的 mspt 尖峰是 `forceload` 一次生出平台周圍 9 個區塊並同步等待,vanilla 也是主執行緒同步生成;其他區域不受影響。)客戶端沒有任何 rubber-band,伺服器沒有 `moved too quickly`。`docs/wp65/elytra_flight.jpg`

### 突襲(`p65_raid.py`,兩個玩家、同一座村莊 (1463,111,-1043) 的鐘旁)
- 兩個玩家真的把 Ominous Bottle 喝下去 → **問題 5**(伺服器完全沒給壞兆頭)。修正後:兩邊都在喝下後得到 `raid_omen`(村莊裡壞兆頭立刻變突襲之兆),約 13~18 秒後第一批襲擊者(4 掠奪者+1 衛道士…),
  之後依波次出現掠奪者/衛道士/女巫/劫掠獸(Kiln 的紀錄:第 1 波 4 掠奪者+1 衛道士、第 2 波 3+3、第 3 波起有劫掠獸與女巫;vanilla 的紀錄一樣的開頭,第 5 波有喚魔者+劫掠獸;Kiln 這次的紀錄在劫掠獸那一波被我截斷,沒看到喚魔者);兩個玩家都得到「Voluntary Exile」(殺了隊長)與「Monster Hunter」。`docs/wp65/raid_wave1.jpg`
- 我讓每波真打 25 秒,剩下的用 `kill` 清掉,所以沒有走到「Hero of the Village」(`kill` 不算擊殺勝利);沒有跑完整場。

### 農場
- **甘蔗(觀察者+活塞)與海帶**:8 組(4 甘蔗、4 海帶),`random_tick_speed 2000`:Kiln 每個遊戲 tick 收 0.089 根甘蔗,vanilla 0.079(+13%);海帶兩邊都很慢(各收到 1 個)。
  Kiln 的一次執行(1/14)觀察者全沒動、活塞沒伸(甘蔗長到 2 格、`powered=false`),同一份腳本重跑 9 次都正常,沒有重現——見「差異」。
- **刷怪塔**:17×17 黑暗平台+四條水道+中央 30 格深坑+漏斗進箱子。**自然生怪的平台**:vanilla 150 秒 0 隻怪(隨機 y 軸讓只佔 3/270),Kiln 也只零星幾隻;**改用刷怪磚(殭屍)**:水道把殭屍推進坑、摔死、漏斗收腐肉,150 秒 Kiln 108 個、vanilla 132 個腐肉(同量級);
  殭屍落下、水流推動、摔落傷害、漏斗收集都和 vanilla 一樣。
- **鐵傀儡農場**:6 個村民關在鐵柵欄格子裡(`last_slept` 設成現在)、旁邊一隻殭屍嚇他們,100 秒內**兩邊都 0 隻傀儡**(vanilla 的觸發在 `VillagerPanicTrigger`,要村民真的恐慌,我的格子沒做到),沒有結論;
  wp59 已經玩過「睡過一夜 → 天亮出傀儡」。

### 其他
- 騎在馬上按 E:**問題 6**。
- 平台、陷阱箱、告示牌等沒有重玩(wp58~wp64 玩過)。

## 發現的問題與修正(全部有測試,拿掉修正測試就失敗)

### 問題 1:疣豬獸自然生怪被「動物的亮度規則」擋掉(已修正)
- 現象:緋紅森林 600 遊戲秒內 vanilla 70 隻疣豬獸、Kiln 0 隻;堡壘 100 比 6。
- 原因:自然生怪的 `isValidPositionForMob` 用 `getWalkTargetValue(pos) >= 0`。Kiln 的 `kiln-sim/src/spawner.rs` 對所有「動物」(疣豬獸在 vanilla 繼承 `Animal`)套用 `lightMagic - 0.5`,而且 `lightMagic` 漏了
  維度的 `ambient_light`(下界 0.1)。`Hoglin.getWalkTargetValue` 在 vanilla 是覆寫過的:**緋紅菌岩 10、否則 0**(跟光無關)。
- 修正:`LevelReader.getLightLevelDependentMagicValue` 補上 `lerp(ambientLight, f, 1)`;新增 `Kind::spawn_walk_target` hook,疣豬獸(緋紅菌岩 10、否則 0)、終界使者、掠奪者、監守者、嘎吱怪(都是 0)用 vanilla 的覆寫值。
- 測試:`kiln-entity/tests/nether_spawn_rules.rs`(3 個);實機:緋紅森林 0 → 92。

### 問題 2:熾足獸不會自然生成(已修正)
- 現象:緋紅森林/堡壘 vanilla 41~48 隻,Kiln 0 隻(`docs/parity-coverage.md` 早就列「strider 無自然生成規則」)。
- 原因:沒有 `IN_LAVA` 放置規則(預設 `OnGround`)、沒有 `Strider.checkStriderSpawnRules`(熔岩柱上第一格是空氣)、沒有 `checkSpawnObstruction` 容許熔岩、`getWalkTargetValue` 的 10/0。
- 修正:`strider.rs` 補 `placement() = InLava`、`check_spawn_rules`、`spawn_in_liquids`、`spawn_ignores_light`。
- 測試:`nether_spawn_rules.rs::a_strider_spawns_in_lava_under_air`;實機 0 → 49/59。

### 問題 3:兩個玩家同時走進終界閘門,第二個永遠留在閘門裡(已修正)
- 現象:兩個真實玩家在同一個 tick 站進第一個閘門:A 被傳到出口島,B 站在閘門裡 12 秒仍沒走;vanilla 兩個都走。
- 原因:Kiln 在「執行傳送」時才檢查閘門的 40 tick 冷卻(vanilla 是在 `entityInside` 才擋「新進入」);第二個玩家的行程被拒絕,但玩家的 `portal_cooldown` 已被設成 10 並且站在裡面一直被補滿,於是再也不會觸發。
- 修正:同一個 tick 內啟動的冷卻不擋(它們都已經過了 `entityInside`);冷卻中被拒絕的玩家清掉傳送程序與傳送冷卻,冷卻結束時自己會走。
- 測試:`kiln-sim/tests/gateway_trips.rs`(2 個:同一 tick 兩人都走;冷卻中進去的人在冷卻結束後走)。

### 問題 4:鞘翅滑翔在伺服器端從來沒有啟動(已修正)——影響最大
- 現象:裝備鞘翅、起跳再按跳躍:客戶端自己進入滑翔姿勢(客戶端自己預測),但伺服器沒有 `FALL_FLYING` 旗標、不消耗鞘翅耐久、煙火火箭用不了(不消耗、不加速);落地後客戶端的 `fallFlying` 永遠為真(伺服器從來沒有改過旗標,所以不會送清除)。
  wp58 的「鞘翅滑翔、煙火火箭加速約 6 格/秒」其實是客戶端單機預測。
- 原因:`kiln-sim/src/glide.rs` 的 `START_FALL_FLYING = 8`(1.21 以前還有蹲下動作的編號)。26.x 的 `ServerboundPlayerCommandPacket.Action` 是 `STOP_SLEEPING, START_SPRINTING, STOP_SPRINTING, START_RIDING_JUMP, STOP_RIDING_JUMP, OPEN_INVENTORY, START_FALL_FLYING`,
  所以是 **6**(`javap` 確認)。既有的測試也用了 8,所以沒發現。
- 修正:改成 6 並在註解寫明順序;測試 `items.rs::elytra_glide_wears_the_wings` 改成 6。
- 實機:`0=-128`、`6=FALL_FLYING` 立刻送給觀看者與自己,落地時 `0=0 6=STANDING`;鞘翅耐久 24;煙火火箭消耗並加速到 32.8 格/秒(上表)。

### 問題 5:喝下不祥之瓶,伺服器沒有給壞兆頭,所以永遠不會有襲擊(已修正)
- 現象:喝下 Ominous Bottle:客戶端畫面有壞兆頭圖示(客戶端自己預測),伺服器上沒有任何效果(`data get entity … active_effects` 沒有、`/effect clear` 說沒有、封包沒有 `UpdateMobEffect`),在村莊裡也不會變突襲之兆。
- 原因:不祥之瓶的效果不是 `on_consume_effects`,而是 `OminousBottleAmplifier`(`ConsumableListener`)的 `onConsume`:`BAD_OMEN`、120000 tick、等級 = 元件值、不顯示粒子。`consume.rs` 的 listener 只做了食物、藥水、燉菜。
- 修正:補上這個 listener。測試 `health.rs::drinking_an_ominous_bottle_gives_bad_omen_of_its_level`。實機:喝完在村莊裡立刻 `raid_omen2`,之後整場突襲。

### 問題 6:騎著馬按 E(物品欄鍵)沒有反應(已修正)
- 原因:`PlayerCommand.OPEN_INVENTORY`(26.x 的 5)沒有處理(`HasCustomInventoryScreen.openCustomInventoryScreen`:騎乘中打開坐騎的畫面)。
- 修正:`carts::open_ridden`,條件與 `mount_info` 的 `saddle_usable`(已馴服、活著、成年)一致;並且它不是「只動自己」的封包,不能走玩家封包的平行路徑(`is_player_packet` 排除)。
- 測試:`mounts.rs::the_inventory_key_while_riding_opens_the_mounts_screen`;實機:E → `HorseInventoryScreen`。`docs/wp65/horse_e.jpg`

### 小差異(沒修)
- `data get block` 對箱子 vanilla 多印一個 `components: {}`、閘門方塊實體 Kiln 多印 `ExactTeleport: 0b` 且 `Age` 不增加(vanilla 160 tick 後 `Age: 159`;客戶端自己數,所以光束動畫的起點可能不同)。
- 隨機刻:`random_tick_speed 2000` 下 Kiln 甘蔗成長快 13%(兩個實作的「有放回」抽法在極端的速度下才會分叉,預設 3 時相差 1e-7),沒有追。

## 差異與還沒做的

- **鐵傀儡農場**:兩邊都沒有觸發(我的格子沒讓村民恐慌),所以「僵屍嚇村民 → 生傀儡」沒有驗證;`spawnGolemIfNeeded` 由 `Villager` 自己的一個方法與 `VillagerPanicTrigger` 呼叫(javap)。
- **刷怪塔的自然生怪**:vanilla 與 Kiln 都幾乎不出怪(平台只佔隨機 y 的 3/270),只驗證了刷怪磚版本;沒有量化的比較。
- **農場一次未重現的失敗**:Kiln 一次執行(14 次中)甘蔗/海帶的觀察者都沒觸發,同一份腳本重跑 9 次都正常;當時 VM 被斷線客戶端壓到負載 18(滑翔啟動的失敗也只出現在這種負載下:`START_FALL_FLYING` 到達時伺服器的 `on_ground` 仍是 true,與 vanilla 同樣的先後規則)。沒有找到根因,不當成 Kiln 問題也不排除。
- **突襲**沒有打到「Hero of the Village」;只比了波次出現與兩邊的組成大方向(精確的波次組成已有向量比對,`parity-coverage.md` 6.x)。
- **豬布林交易**只比了流程,沒有量化分布((vanilla 那次拿到的是乾燥幽靈,26.3 的交易戰利品表有它))。
- 睡覺時「別人眼中的我是睡著的」在腳本裡兩邊都沒成功(A 沒真的睡上床,兩邊相同),留待下次。
- 下界走路(穿洞穴)和傳送門被怪物打斷沒有測;下界頂上的飛行是在下界天花板上方。
- 這次沒做:磚塊/基岩的下界要塞整合測試(wp59 做過)、骷髏/凋零骷髏頭的掉落、鞘翅的耐久與修補。

## 最後驗證

見下方「最終驗證」(跑完才寫入)。
