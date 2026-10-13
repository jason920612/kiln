# wp59:第二次真實客戶端遊玩(wp58 沒走到的部分)

目的:用官方 26.3 客戶端(跑在 VM 的 Xvfb + llvmpipe 上,由 `tools/wp58/ClientCtl.java` 這個 java agent 驅動;客戶端不在 Windows 上跑)
玩 wp58 沒玩到的東西:紅石農場、活塞門、漏斗熔爐陣列、釣魚、村民(繁殖/交易/鐵農場)、突襲、海底神殿、試煉空間、woodland 林地府邸、
終界城(船/鞘翅)、下界(獄髓/堡壘/傳送門連結)、多人。
伺服器 `kiln` release(`KILN_GENERATOR=noise`、種子 59、`KILN_DATAPACK=~/kiln/work/generated`),世界 `~/wp59/world`,port 25585。
**指令只用在「佈置」**(造平台、`fill`、`summon`、`give`、`tick rate`、`time set`、為了不被怪物清除而下的 `effect give resistance`),
實際操作(點方塊、開 GUI、交易、戰鬥、釣魚、放東西、點拉桿/按鈕、走進傳送門、騎馬)都走客戶端。

## 環境與工具(相對 wp58 的變動)

| 項目 | 做法 |
| --- | --- |
| 驅動 | `tools/wp58/ctl.py` 讀 `KILN_PLAY_DIR`(預設 `wp58`,這次 `wp59`)。`ClientCtl.java` 增加:`offers`(交易列表)、`selecttrade i`、`item slot`;狀態裡有 `effects`/`xp`/`armor`;實體清單先列生物再列掉落物(以前掉落物多時生物被 60 筆上限擠掉),並附 `hp`/`baby`/村民職業/物品 id。`tools/wp58/build_agent_vm.sh` 在 VM 上直接編譯 agent |
| 伺服器 | `tools/wp58/wp59_start.sh`(console = fifo `~/wp59/console`,log `~/wp59/server.log`);常用輔助函式 `tools/wp58/p59.py`(`sv`/`fill`/`kill_nearest`/`fish`/`free_spot`/`alive`…,在 VM 上 `KILN_PLAY_DIR=wp59 PYTHONPATH=tools/wp58 python3 -` 餵腳本) |
| 場地 | y=200 的 115×109 石頭/草方塊空中平台,農場蓋在上面;`tick rate 100` 用來等待(實測 100.0 tps,客戶端與伺服器都撐得住) |
| 機器人 | `kiln-bot`:`--behavior survival --roles explorer,miner,builder,redstone`、`--behavior circle --chat-interval` |
| 截圖 | VM 的 Xvfb `:99`(只有遊戲畫面);精選的在 `docs/wp59/*.jpg`,全部在 VM 的 `~/kiln/work/wp59/` |

## 流程與結果

### 1. 紅石農場

- **甘蔗(觀察者+活塞)**:3 組(沙+水+甘蔗,觀察者看第 2 格,紅石線接活塞)。`random_tick_speed 300` 下甘蔗一長出第 2 格就被活塞打斷,掉落物落在農場周圍;實際走過去撿到 46 根。
- **海帶(觀察者+活塞,玻璃水柱)**:水柱 6 格,觀察者看第 3 格,活塞從側面打斷。海帶高度被穩定控制在 2 格,掉落物浮到水面(`docs/wp59/farm_kelp.jpg`)。我第一次接線錯(活塞放錯邊)所以海帶長過頭,不是 Kiln 的問題。
- **牛/雞圈**:餵小麥/種子繁殖(右鍵),出現小牛(`cows2.jpg`);桶擠牛奶;殺牛得到牛肉、皮革。雞圈地板是 3×3 的漏斗匯到發射器,按按鈕把蛋丟進圈裡(真的按)。→ 找到**問題 1(雞不下蛋)**。
- **普通紅石細節**:`setblock redstone_wire` 放出來的是「點」不會接活塞;客戶端真的放的紅石線是十字(`east/north/south/west=side`)。與 vanilla 一致。

### 2. 2×2 活塞門

4 個活塞(伸出的活塞頭當門),兩側各一根拉桿(拉桿 → 石頭 → 紅石線,另一格吃石頭的強充能);**客戶端真的點拉桿**:關起/打開。
活塞頭伸進玩家站的格子,玩家被推出(z -35.5 → -36.31),與 vanilla 一致。

### 3. 漏斗熔爐陣列

4 座熔爐,上面各一個向下的漏斗(從輸入線取),側面一個漏斗(燃料),下面一條漏斗線收成品。用 GUI 把 29 生鐵/16 生銅/8 生金/19 牛肉塞進輸入箱,煤塞進燃料箱:全部產出。
這個陣列只有第一座熔爐在工作:漏斗每 tick 都在抽,第一座熔爐的輸入格吃滿(64)才放行;與 vanilla 行為一致(標準做法要加過濾)。
**物品分類器(comparator 鎖定的那種)沒有做**:試著用「塞滿不可堆疊物的漏斗」當過濾器只驗證了漏斗只收能與現有格子堆疊的物品;完整的比較器鎖定分類器沒蓋完。

### 4. 釣魚

幸運之海 III + 餌釣 III 的釣竿,真的拋竿、看浮標下沉(客戶端實體的 y)再收竿:約 85 次拋竿,鮭魚/鱈魚/河豚/熱帶魚、睡蓮(垃圾)、**寶藏**:損壞嚴重的弓(力量 IV、擊退 I)、釣竿(幸運之海 II + 消失詛咒)。
開闊水域檢查有效:第一個水池只有 5×5、浮標落在邊上,拿不到寶藏;擴成 11×11 後有(vanilla 也這樣)。

### 5. 村民

- 小屋(6 張床、講台/堆肥桶/鍛造台/製圖台/高爐):5 個無職業村民在 1 秒內各自領到職業(`trade_lib.jpg`)。
- **交易**(MerchantScreen,真的點):圖書管理員新手「24 紙 → 1 綠寶石」,賣 5 次後(經驗 10)升到學徒,新增燈籠、書→綠寶石;「綠寶石+書 → 附魔書」成交(拿到詛咒附魔書,vanilla 的村民也賣詛咒)。
- **繁殖**:給麵包(村民 `CanPickUpLoot` 必須是 true;`/summon villager` 不帶 NBT 的村民和 vanilla 一樣是 false,`Mob.readAdditionalSaveData` 預設 false,我用 javap 確認過),2 個有 ≥12 食物點的村民 + 夠多的床,10 秒(rate 100)內生出嬰兒。
- **鐵農場**:5+ 村民、整晚睡過,天亮出 1 隻鐵傀儡(沒睡過是不會出的,與 vanilla 一致);真的打死它得到 5 鐵錠、2 罌粟。

### 6. 突襲(raid)

不祥之兆 → 突襲之兆(30 秒)→ 突襲開始。真的打了整場:掠奪者、劫掠者(ravager)、女巫、喚魔者(evoker)+ 惱鬼(vex)、衛道士,最後「Hero of the Village」(`hero.jpg`)。
喚魔者掉不死圖騰,隊長掉不祥之瓶,ravager 掉鞍。
過程中我被劫掠者/衛道士殺過很多次(死亡會清掉藥水效果,要在重生後再給抗性),`keep_inventory` 保物品。

### 7. 海底神殿

- 客戶端渲染正常;水下 64 隻守衛者(神殿內自然生怪的上限內,vanilla 也是塞滿)。
- 殺死元素守衛者:掉濕海綿、海晶碎片、鱈魚、**潮汐盔甲紋飾模板**。濕海綿放進熔爐烤乾(GUI)得到海綿;海綿放進水池吸水變濕。
- **挖礦疲勞**:元素守衛者詛咒的特效畫面與挖礦疲勞 III(6000 tick)(`fatigue.jpg`)。
- → 找到**問題 3(神殿沒有元素守衛者)**。

### 8. 試煉空間(trial chamber)

在 (-288, y≈-22, 0):進度「Minecraft: Trial(s) Edition」;試煉刷怪磚(bogged、洞穴蜘蛛)真的打,狀態 `active → waiting_for_reward_ejection → ejecting_reward → cooldown`,獎勵(熟雞、…)吐出;試煉鑰匙開寶庫(vault):`active → ejecting → inactive`,得到箭、風彈、綠寶石,消耗 1 把鑰匙;不祥寶庫要不祥鑰匙(`trial1.jpg`)。

### 9. 林地府邸

(368, -928):19 個衛道士、5 個喚魔者(persistent),真的殺衛道士得到綠寶石。→ 找到**問題 4(府邸沒有居民)**。

### 10. 終界城

`locate` 找到 13 座城;其中兩座有船(其他的只有塔)。在 (-2160, 1664) 的船:龍頭、艙房裡**兩個箱子夾著放鞘翅的展示框**(`end_ship.jpg`);45 隻潛影貝(包括擋在展示框前面的那隻),被打中會得到懸浮(levitation);殺掉它後打展示框(要站近,實體攻擊距離 3 格),**鞘翅掉出來**並撿到;箱子是終界城戰利品(鑽石、金錠、鑽石矛…)。
潛影殼 + 箱子合成潛影盒(GUI),放東西、挖起來,內容物保留在物品裡。wp58 說的「找不到船」只是那座城沒有船。

### 11. 下界

- **堡壘**(-224, ~, 336):豬靈/豬靈蠻兵/疣豬獸;3 個箱子(金石、鐵粒、金胡蘿蔔、岩漿膏、十字弩…);打死豬靈蠻兵。
- **獄髓**:在 y=16~19 找到遠古遺骸(Kiln 的下界地形有),挖(真的挖,鑽石鎬)3 個;熔爐燒成獄髓碎片(3 個);合成台合成獄髓錠(4 碎片 + 4 金錠,GUI);鍛造台升級鑽石鎬 → 獄髓鎬(`smithing.jpg`)。
- **傳送門連結**:A (-60, 201, 40) → 下界新門 (8, 58, 4);第二個門 B (-36, 201, 40)(距離 24 格 = 下界 3 格)→ **同一個**下界門;從下界回來走到離它最近的 B。用「站在門裡面 4 秒」(走過去不夠),與 vanilla 相同。
- 不死圖騰:偏手拿著從 130 格高落下,圖騰觸發,進度「Postmortal」,效果再生 II/防火/吸收 II。

### 12. 其他 GUI / 實體

- 鐵砧(附魔書合併 + 改名,等級 30 → 25)、信標(金字塔 + 選「迅捷」+ 付鐵錠 → 速度效果,`beacon.jpg`)、馬(鞍 + 騎乘 5 秒 41 格)、動力鐵軌(8 格/秒)、殭屍村民治療(黃金蘋果 + 虛弱,要在有屋頂的小屋裡——白天會被曬死;約 55 秒(rate 100)後變回圖書管理員,職業與等級保留,進度「Zombie Doctor」)、暗室刷怪(黑暗的 11×11 石室,每秒都有怪;水泡過的地板不會刷)。

### 13. 多人

`kiln-bot` 4 個生存玩家(探索者/礦工/建築師/紅石)與 2 個 circle 玩家在我旁邊:聊天、玩家實體(`mates.jpg`,看得到名牌與玩家模型)、加入/離開訊息、機器人真的挖/放(要先 `op`,而且要在機器人**已連線後**才 `op`:`op Bot0` 對未連線的名字會 op 小寫的 `bot0`,離線 UUID 不同,與 vanilla 一樣)。機器人預設在地表找「最高的地面」蓋房子,所以我的 y=200 空中平台下面它們站不到(`out of reach`),把它們放到自然地形後正常。
伺服器 mspt:一個玩家 + 4 個機器人時平均約 3~4 ms。

## 發現的問題與修正

### 問題 1:雞不會下蛋(已修正,有測試)

- 現象:雞圈裡 `EggLayTime` 倒數到 0 會重設,但沒有任何蛋掉出來(漏斗、發射器全空)。
- 原因:26.x 的 `gameplay/chicken_lay` 用 `entity_properties`(`this`)的 `minecraft:chicken/variant` 元件選蛋(白/棕/藍)。Kiln 的 `DeathContext`(`kiln-sim/src/mobs.rs`)的 `this` 沒有任何元件,`Components` 判斷全部不成立 → 空掉落。
- 修正:`DeathContext` 新增 `components`(`kiln_entity::mob::variant_components`),`entity_matches` 處理 `Components`;`loot_drop`(下蛋、剪毛)改成查詢實體取得種類與元件;死亡掉落、剪毛、`/loot` 的來源實體也帶上。
- 測試:`kiln-sim/tests/animals.rs::a_chicken_lays_the_egg_of_its_variant`(三種變種各下對應的蛋;需 `KILN_DATAPACK`)。實機:修正後 10 秒內蛋進了發射器。

### 問題 2:盔甲/裝備屬性沒有同步給客戶端,護甲條是空的(已修正,有測試)

- 現象:穿上鑽石全套,伺服器 `attribute armor get` = 20,客戶端 `getArmorValue()` = 0,HUD 沒有護甲條。
- 原因:`kiln-sim/src/combat.rs` 只把 6 個「藥水效果會改的」屬性(速度、攻速、摔落距離、最大血量、吸收、幸運)送給客戶端(`EFFECT_SYNCED`),vanilla 的 `isClientSyncable` 有 33 個;而且換裝備時沒有標記「屬性變了」。
- 修正:`EQUIPMENT_SYNCED`(護甲、護甲韌性、挖掘效率、水下挖掘速度、水中移動效率、氧氣加成)加入同步;`tick_combat` 偵測到裝備變動時設 `attributes_dirty`。
- 測試:`kiln-sim/tests/combat.rs::armor_and_tool_attributes_reach_the_client_when_equipment_changes`(穿上 4 件後 `armor` 修飾符 4 個、合計 20;脫掉胸甲後剩 3 個)。實機:脫/穿胸甲,客戶端 armor 20 → 12 → 20,HUD 有護甲條(`armor3.jpg`)。
- 沒補的:vanilla 其他可同步屬性(`entity_interaction_range` 的創造模式修飾符、`gravity`、`jump_strength` 等);目前沒看到客戶端因此錯位。

### 問題 3:海底神殿沒有元素守衛者(已修正,有測試)

- 現象:`/locate` 找到的神殿,方圓 100 格沒有元素守衛者(vanilla 一座神殿有 3 隻:兩個側翼、一個頂樓房)。
- 原因:`kiln-worldgen` 的神殿(`legacy/monument.rs`)沒有 `OceanMonumentPiece.spawnElder`,結構生成只做「亂數抽取以保持一致」。
- 修正:兩種側翼(座標 (11,2,16) / (11,5,13))與頂樓 (6,1,6) 加 `spawn_elder`(`add_entity` 到區塊的生成實體清單,健康 80,附 `finalizeSpawn` 的亂數抽取)。
- 測試:`kiln-worldgen/tests/pipeline.rs::an_ocean_monument_comes_with_its_three_elder_guardians`(種子 59、(320,-224) 的神殿剛好 3 隻)。實機:新神殿 (1824, 2080) 有 3 隻,挖礦疲勞、掉落都正常。

### 問題 4:林地府邸沒有居民(已修正,有測試)

- 現象:府邸沒有喚魔者、衛道士、悅靈(`Mage`/`Warrior`/`Group of Allays` 資料標記只清掉標記、抽亂數,不生成)。
- 修正:`legacy/mansion.rs` 生成持久(`PersistenceRequired`)的喚魔者、手持鐵斧的衛道士、1~3 隻悅靈。
- 測試:`pipeline.rs::a_woodland_mansion_is_inhabited`(種子 59、(368,-928) 的府邸至少 1 個喚魔者和 1 個衛道士)。實機:19 個衛道士、5 個喚魔者,殺衛道士掉綠寶石。
- 悅靈群組只出現在某些房間,測試沒要求。

### 問題 5:沼澤小屋沒有女巫和黑貓(已修正,有測試)

`kiln-worldgen/src/structure/legacy/scattered.rs`:只記錄「應該生成」的旗標。現在在 (2,2,5) 生成持久女巫與黑貓(`variant: minecraft:black`)。測試 `pipeline.rs::a_swamp_hut_has_its_witch_and_cat`(種子 59、(-704,880))。

### 問題 6:海底遺跡沒有溺屍(已修正,有測試)

`structure/templated/ocean_ruin.rs`:`drowned` 資料標記只換成水/空氣,不生成。現在生成持久溺屍。測試 `pipeline.rs::an_ocean_ruin_has_drowned`(種子 59、(160,480) 的冷水遺跡)。

(6 個修正都在結構/區塊生成的實體部分,vanilla 生成這些實體的其他來源——終界城的潛影貝與展示框、礦坑的運輸礦車、`StructureTemplate` 裡的實體——Kiln 本來就有。Kiln 生成實體的 `finalizeSpawn` 只做亂數抽取,不做裝備與嬰兒機率,見 `finalize_spawn_draws`。)

## 不是問題但容易誤判的事

- 雷雨天白天也會在開闊地生怪(26.x 天氣層把天空亮度壓到 4);我一開始看到平台上 70 隻怪以為是 bug,關了 `advance_weather` 後恢復正常。
- `Unable to play empty soundEvent: minecraft:entity.salmon.ambient`:vanilla 資產裡這個聲音本來就是空的(`sounds: []`),vanilla 也會有這條警告。
- 伺服器 WARN `KilnTest moved too quickly! [0, -11…-26, 0]` 出現 3 次,都在我用 `/tp` 到高空自由落下時(客戶端是軟體 OpenGL,tp 後常卡幾十 tick),沒有對應的 vanilla 向量,不當成 Kiln 問題。
- 暗室刷怪失敗的第一版是因為我放的水把整個地板淹了(水裡不會刷怪)。
- 漏斗熔爐陣列只用一座熔爐、bot 在空中平台下「out of reach」、`op` 大小寫:見上。
- 村民撿麵包:`CanPickUpLoot` 為 false 的 `/summon` 村民不會撿。

## 最終驗證(VM,分支 wp59-playthrough2)

- `cargo test --workspace --release --no-fail-fast`:不帶 `KILN_DATAPACK` 全綠(EXIT 0);帶 `KILN_DATAPACK=/home/test/kiln/work/generated` 全綠(EXIT 0)。
  第一輪(沒有 `--no-fail-fast`)在 `kiln-sim/tests/crowd_golden.rs` 停住:4 個情境的「封包串雜湊」變了,狀態雜湊沒變——原因是問題 2 的修正讓登入的 Update Attributes 封包多列了 6 個裝備屬性(與 wp45/wp54 一樣屬於預期的重新錄製);我把 4 個封包雜湊重錄(`0x45379799a318b2a4`、`0x438e8bdeaed2c6b9`、`0x2295b54ae462e3fc`、`0x499303e49fdafb6b`)並在檔頭註記 wp59。
- `kiln-sim --test determinism`:通過。
- `kiln-entity --test mob_parity`(`work/m6-mobs2/vectors.jsonl`,先備份成 `~/wp59/vectors.jsonl.bak`,沒有新增向量):**1437 個情境全部與 vanilla 一致**(568011 個 mob 狀態)。
- `tools/parity_suites.py --skip-workspace`:102 個 suite 行,全部 exit 0。
- 負向檢查:把問題 1 的 `Components` 判斷、問題 2 的 `EQUIPMENT_SYNCED` 拿掉,對應的新測試 `a_chicken_lays_the_egg_of_its_variant`、`armor_and_tool_attributes_reach_the_client_when_equipment_changes` 都會失敗(用臨時分支 `wp59-negcheck` 在 VM 上跑,已刪除)。
  結構生成實體的測試本來就是「有沒有實體」,拿掉修正必然失敗。
- VM 上 `~/wt/wp59-playthrough2/target` 與臨時的 `wp59-negcheck` 已刪;客戶端、伺服器已停。

## 還沒做到 / 後續

- **物品分類器**(比較器鎖定的 hopper 分類器)沒有蓋完。
- 沒做:治療後的交易折扣數值、流浪商人、蜜蜂、馬繁殖、地圖/製圖台、織布機。
- 神殿/府邸的實體只補了「怎麼生」,不做 `finalizeSpawn` 的裝備、嬰兒機率;`kiln-entity` 的 mob_parity 不涵蓋結構生成。
- 屬性同步只補了與裝備相關的 6 個,其他 vanilla `isClientSyncable` 屬性仍可能缺。
