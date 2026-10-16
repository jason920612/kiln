# wp68:第六次真實客戶端遊玩(26.x 最新內容:硫磺方塊、銅傀儡、快樂恐魔、騎乘大改、試煉密室、考古…,並排 vanilla 26.3)

分支 `wp68-playthrough6`(從 main `3c5ed11e` 開出)。做法同 wp58~wp65:官方 26.3 客戶端跑在 VM 的 Xvfb 上,由 `tools/wp58/ClientCtl.java`(java agent)驅動;
**所有建置、測試與遊玩只在 VM 上做**(Windows 沒有跑 cargo、也沒有跑客戶端)。伺服器:Kiln(`KILN_GENERATOR=noise`、種子 68、port 25589)與 vanilla 26.3 參考伺服器(同種子、port 25595)。
指令只用在佈置(平台、`give`、`tp`、`summon`、`setblock`、`fill`、清場);右鍵、攻擊、開 GUI、騎乘、滾輪、拿放物品、走路、跳躍、釣魚式的動作都走真實客戶端。
Mojang 的資料不在 repo;截圖只有 Xvfb 的遊戲畫面,放在 `docs/wp68/`(shelf、ghast、golem、creaking、bundle、vault、sulfur)。

## 工具(相對 wp65 的變動)

| 項目 | 做法 |
| --- | --- |
| 驅動 | `ClientCtl` 新增 `scroll DY`(滑鼠滾輪)、`guihover SLOT`(滑鼠移到格子上)、`bundlesel SLOT\|inv N`(讀客戶端眼中束口袋的選取) |
| **分割模式**(新) | `switch.sh K split`:演員 `KilnTest`(agent 25999)連 Kiln、**另一個也叫 `KilnTest` 的客戶端**(agent 26000)連 vanilla;`par.sh script.py` 把**同一支腳本同時**丟給兩邊(輸出 `out_K.txt`/`out_V.txt`,事實記在 `facts/<場景>_<K\|V>.txt`)。兩邊同一時刻做同一件事,所以相位雜訊相同 |
| 雙客戶端同伺服器 | `switch.sh K\|V both`:演員與 `Obs` 連同一個伺服器(多人場景:保險庫各拿各的獎勵、兩人坐快樂恐魔) |
| 腳本 | `p68.py`(`scenario/F/rec_start/spy_diff/use_on_entity/ents_of/alive_auto`…)、`p68_small`(黃金蒲公英、人偶)、`p68_sulfur`、`p68_copper`、`p68_copper2`(箱子、層架、工具、避雷針)、`p68_ghast`、`p68_spear`、`p68_mounts`、`p68_trial`、`p68_vault2`、`p68_breeze`(風彈、重錘、微風、沼澤骷髏)、`p68_brush`、`p68_crafter`、`p68_bundle`、`p68_wolf`、`p68_sniffer`、`p68_creaking`、`p68_persist`+`persist_cmp.py`/`persist_kv.py`(重啟持久化)、`p68_shots`(截圖場景)、大量 `x_*.py` 小實驗 |
| 啟動/重啟 | `wp68_start.sh`、`wp68_vanilla.sh`、`wp68_kiln_keep.sh`/`wp68_vanilla_keep.sh`(保留世界重啟)、`restart_k.sh`(重建+重啟 Kiln+重接演員)、`restart_both.sh`、`vm_client68.sh`、`vm_observer68.sh` |
| 負向檢查 | `tools/wp68_neg.py`:把每個修正用一行文字編輯關掉,對應測試必須失敗 |

踩到的坑(不是 Kiln 的問題):`pkill -f 'mc-client/game2'` 的樣式出現在自己的 ssh 指令列上,殺掉自己;編譯時記憶體吃緊,第二個客戶端的 JVM 被 OOM 殺掉(之後編譯前先看 `free -m`);
`/kill` 史萊姆/硫磺方塊會分裂,分裂出來的小的還帶著 tag(`kill @e[tag=…]` 之後仍有);長 `tick sprint` 期間幻翼會生出來、快樂恐魔幼體會飛走;
`fill … air` 把平台一起清掉,玩家掉進虛空;vanilla 玩家死掉要點 480×270 視窗的重生鈕;`setblock` 的 crafter 狀態字串;Kiln 把死掉的玩家(Health 0)存下來,重連時直接在死亡畫面,害我以為保險庫對第二位玩家沒反應;
Python 在 Windows 的預設編碼是 cp950,改檔要指定 `encoding='utf-8'`。

## 先驗證的事(Kiln 與 vanilla 完全一致)

- **試煉密室**:`/place structure minecraft:trial_chambers` 同種子同座標產生相同房間;試煉刷怪籠整個週期 `active → waiting_for_reward_ejection → ejecting_reward → cooldown`(冷卻 36000 刻),保險庫 `unlocking → ejecting → inactive`、`rewarded_players` 逐位玩家記錄。
- **矛**:刺擊(左鍵)與衝刺蓄力(按住右鍵跑)的傷害、擊退、耐久、音效逐項相同。
- **鸚鵡螺/殭屍鸚鵡螺**:馴服、鞍、護甲、游泳、衝刺;**駱駝/駱駝屍殼**:騎乘與衝刺;**沙/礫考古**:階段計時、同一戰利品種子下的掉落;**犰狳刷鱗甲**;**合成器+拉桿**;**束口袋**(滾輪選取、插入、取出,見問題 8);**狼鎧**裝上、吸收、耐久;**銅傀儡**建造、分類進箱子、塗蠟/斧頭除蠟、氧化、變雕像;**人偶**;**快樂恐魔**水化、成長、掛具、兩人同騎、操控、上升;**避雷針**被雷擊的充能 8 刻與紅石燈(三個高度的取樣兩邊相同);**閃電打在銅塊上**(氧化的變回、上蠟的不變);**創角心與樹脂**(夜晚自然生成創角怪、被看著時不動、打它心會流樹脂、拆心創角怪死亡、樹脂團貼牆、爐子燒樹脂磚,兩邊的音效與粒子清單逐項相同,240 個軌跡粒子對 240 個);
  **風彈**直接往腳下丟的最高點(修正後兩次 206.07/206.08 對 206.07/206.08);**微風**25 秒射 7 顆風彈對 7 顆;**沼澤骷髏**剪毛(掉蘑菇、`sheared: 1b`、第二次剪沒有);**銅箱子**開關音效與方塊事件、雙箱、除蠟、氧化(11 種銅方塊 25 秒氧化到最後階段兩邊相同);**兩個玩家各開一次保險庫**(各消耗 1 把鑰匙、各拿各的獎勵、`rewarded_players` 兩人、第二次開被拒絕音效、之後 `inactive`;含不祥保險庫);**不祥之兆**(一個玩家帶著 `trial_omen` 站進去,刷怪籠 1 秒內變 `ominous=true`,兩邊的狀態序列與效果清單相同)。

## 玩了什麼、找到並修好的問題

| # | 現象(真實客戶端看到的) | 原因 | 修正 | 迴歸測試(沒有修正會失敗) |
| --- | --- | --- | --- | --- |
| 1 | `/summon mannequin` 讓客戶端直接斷線(Network Protocol Error) | 人偶不是 Mob,Kiln 卻照 Mob 送 entity data 欄位 15(mob flags) | 只在 `kind.is_mob()` 時送;幼體才送 `AGE_LOCKED` | `mobs.rs::every_mob_sends_only_fields_of_its_entity_type`(逐一檢查每種實體送出的欄位都屬於它的型別) |
| 2 | 用空桶點硫磺方塊:客戶端預測桶變滿,伺服器沒拿走方塊,放下去才發現桶是空的 | 硫磺方塊桶(取起、放回、名稱/血量/年齡/身體裝備存進桶)伺服器端完全沒做 | `sulfur_cube.rs`:`bucket_pickup/save_to_bucket/apply_bucket`;`fish.rs::bucket_mob`、`buckets.rs`、`mobs.rs::bucket_release` | `kiln-entity/tests/sulfur_bucket.rs` |
| 3 | 黃金蒲公英餵幼牛/幼豬/幼村民:客戶端預測音效,伺服器沒鎖年齡、沒消耗 | 只有蝌蚪/馬/熊貓有實作,其他可鎖年齡的幼體沒有;鎖與強制成長的粒子沒送 | `interact.rs::ageable_dandelion`(`#cannot_be_age_locked` 之外的所有幼體)、`breed.rs` 粒子 | `kiln-entity/tests/age_lock.rs` |
| 4 | 乾燥恐魔澆水孵化:方塊消失、什麼都沒生 | `Effect::HatchGhastling` 是空的 | 生出幼體快樂恐魔(朝向、`entity.ghastling.spawn` 音效) | `kiln-sim/tests/ghastling.rs`(連玩家收到 AddEntity 都檢查) |
| 5 | 幼體在客戶端**永遠不會長大**(伺服器 Age 到 0,客戶端還是小的) | entity data 回到預設值時沒送(只送非預設值,vanilla 的 `sendDirtyEntityData` 會把改回預設的也送) | `EntityData::dirty_since`:只送變動的與「回到預設」的欄位;`entities.rs` 追蹤上一份 | `kiln-proto` 單元測試 `a_value_back_at_its_default_is_reset_in_the_dirty_entries`、`kiln-sim/tests/entity_data_reset.rs` |
| 6 | 騎著快樂恐魔時對它 `/data merge` 會把乘客甩下 | merge 重載實體時沒保留乘客/載具連結 | `command_data.rs` 保留 `passengers`/`vehicle` | `kiln-sim/tests/data_merge_riders.rs` |
| 7 | `/item replace entity @e[type=horse] saddle with …`、`armor.body`:「no such slot」 | 坐騎額外裝備欄位沒對應 | `mob_extra_ordinal`(saddle 106 → 7、armor.body 105 → 6)+ ext `set_extra_equipment` | `kiln-sim/tests/item_replace_mount_slots.rs` |
| 8 | 海龜蛋裂開/孵化、嗅探獸蛋、鸚鵡螺鞍的音效客戶端不認得 | 3 個寫錯的音效 id | 改正;**新增整個原始碼的音效 id 掃描測試** | `kiln-sim/tests/sound_ids.rs` |
| 9 | 駱駝的 entity data 帶著「已馴服」旗標 | `AbstractHorse` 旗標位元(駱駝永遠算馴服,vanilla 不送) | 只在進食時送 | `mobs.rs::a_camel_does_not_send_the_tame_flag` |
| 10 | 殺羊不掉羊毛(顏色)、殺小史萊姆不掉黏液球、骷髏殺苦力怕不掉唱片、著火的牛不掉熟牛肉、殭屍騎士的載具判斷全錯 | 死亡戰利品的 `entity_properties` 謂詞(`type_specific/sheep`、`cube_mob`、`components`、`equipment`(攻擊者)、`vehicle`、`entity_type`(攻擊者))一律回答「否」;`furnace_smelt` 是空的 | `DeathContext` 帶 `TypeSpecific`/`DeathExtra`;`mob/mod.rs::variant_components` 加羊毛顏色;`smelt()` 查熔爐配方 | `kiln-sim/tests/death_loot_specific.rs` |
| 11 | 束口袋:滾輪選了某一疊,右鍵取出的還是第一疊 | `bundle_item_selected` 封包沒處理,`selectedItem` 不存在 | `BundleContents(pub Vec<…>, pub i32)`、`toggle_selected`、`remove_one` 依選取、`handle_select_bundle_item` | `kiln-inventory` 單元測試 `the_selected_stack_is_the_one_removed` |
| 12 | 嗅探獸蛋裂一次就停住,永遠不孵化 | `onPlace` 在**每次狀態改變**都會跑(下一次裂開是從這裡排程的),Kiln 擋成只在首次放置跑 | 拿掉 `!same_block` 守衛 | `kiln-sim/tests/sniffer_egg.rs`(同時驗證 vanilla 的 3 階段) |
| 13 | **下雨天蜂巢裡的蜜蜂照常飛出**(vanilla 整場雨蜜蜂都待在巢裡) | 26.x 的 `bees_stay_in_hive` 是環境屬性:時間軸的夜晚,**加上天氣層(雨或雷的等級 > 0 就設為 true)**;Kiln 只有夜晚 | `Weather.weather_layer`(雨/雷等級 > 0),`beehive.rs::bees_stay_in_hive` | `kiln-sim/tests/bees_rain.rs`(下雨時準備好的蜜蜂留在巢裡,放晴後飛出) |
| 14 | **木質層架(shelf)完全沒有功能**:右鍵沒反應(vanilla 把物品放上去、空手拿回、通電後換整排快捷欄) | 只有方塊狀態,沒有方塊實體、沒有使用、沒有鏈結 | 新增 `BeKind::Shelf`(3 格、`align_items_to_bottom`)、`kiln-blocks/behaviour/shelf.rs`(通電、`side_chain` 三格鏈結、`SideChainPartBlock` 的連接/斷開、放置狀態、拆除)、`kiln-sim/shelf.rs`(點擊落在哪一格、單格交換、整排換快捷欄:順序為「自己、左邊、右邊」、音效與遊戲事件、客戶端同步、比較器 = 三格是否有物)、空手也走 `useItemOn` | `kiln-sim/tests/shelves.rs`(2 個:單格交換/取回/破壞掉落、三個連鎖層架的 `side_chain` 與換快捷欄順序);實機:層架內容、通電與換快捷欄的 `data get`、音效,與 vanilla 逐項相同 |
| 15 | **風彈/TNT 的擊退**:玩家腳下丟風彈,Kiln 比 vanilla 高 0.85 格(206.93 對 206.08);行進中被炸飛的方向也不同 | Kiln 對被炸的玩家送 `SetEntityMotion`(**取代**客戶端自己的速度),vanilla 在爆炸封包裡帶 `playerKnockback`(客戶端**加到**自己的速度);粒子和音效也是分開的封包;還多抽了兩次關卡亂數(音調)——vanilla 的音調是客戶端抽的 | 所有實體爆炸(TNT、苦力怕、風彈、微風…)改送 `ClientboundExplodePacket`(粒子、音效、方塊數、該玩家的擊退;旁觀者與飛行中的創造玩家不收);伺服器端玩家速度也加上衝量(受傷後的 `markHurt` 動作封包才對) | `kiln-sim/tests/explosion_packet.rs`(3 個:擊退在封包裡、腳下爆炸向上 > 0.8、不另送第二個動作封包);實機兩次 206.07/206.08 對 206.07/206.08 |

另外三個測試輔助(不是 Kiln 錯誤,是測試做錯):蒲公英與幼體的年齡計時、`Size` 是「大小 − 1」、幽靈生成期間 `tick sprint` 的干擾。

### 銅(優先 1)
- **銅傀儡**:銅方塊+雕刻南瓜在地上蓋出傀儡(真實右鍵放置)、分類(把銅箱子裡的東西依箱內物品搬到旁邊的箱子,100 秒的狀態序列、音效、方塊事件)、蜜脾塗蠟、斧頭除蠟、氧化階段、變雕像、黃金蒲公英(見問題 3)。
- **銅箱子**:放置、雙箱合併(`type=left/right`)、開關音效、塗蠟(蜜脾右鍵走的是箱子的使用,和 vanilla 一樣先開箱子)、雙箱拆掉掉落;**銅方塊氧化**:11 種(箱子、雕像、避雷針、燈泡、門、活板門、燈籠、浮雕、柵欄、鏈…)`random_tick_speed 4096` 25 秒後全部到最後階段,兩邊相同(銅火把不氧化,兩邊相同)。
- **層架**(見問題 14)、**避雷針**、**閃電+銅**(上面)。**工具與護甲**:同一組元件資料(攻擊傷害、耐久);嘗試用實機量一擊傷害,但測試本身的時機雜訊太大(殭屍血量讀值錯位),**沒有拿來下結論**,列在「沒做」。

### 快樂恐魔(優先 2)
乾燥恐魔放置、澆水、孵化(問題 4)、雪球餵食、冰塊餵食、幼體長大(問題 5,實機看到客戶端的幼體真的變大)、掛具(染色)、**兩個真實客戶端同騎**(A 騎上、B 騎上;操控、上升;下坐騎時的位置差是客戶端量化)、栓繩。**最多 4 人同騎沒做**(只有兩個客戶端)。

### 騎乘大改(優先 3)
矛、鸚鵡螺/殭屍鸚鵡螺(馴服用 `/item replace` 補鞍與護甲才能繼續,見問題 7)、駱駝屍殼、乾旱(parched)行為(兩邊的差異在雜訊內)。

### 試煉密室(優先 4)
試煉刷怪籠、保險庫、不祥保險庫、不祥之兆、微風、風彈偏轉、沼澤骷髏剪毛、重錘:**重錘(plain)從 12 格高掉下去對殭屍造成 11 點,兩邊相同**;密度/穿透/風爆的傷害讀數受「何時按下攻擊」影響(下落距離每次差 1~2 格),**沒有拿來比對**。

### 較舊的內容(優先 5)
嗅探獸(蛋裂 3 階段、孵化、餵火炬花種子兩次、成長,修正後逐項相同)、考古(刷子)、合成器、犰狳、狼鎧、束口袋(問題 11)、創角怪與樹脂(上面)。

### 重啟後的持久化(優先 6)
`p68_persist.py setup` 在**兩個伺服器**同時蓋出狀態,`dump before` → 兩個伺服器都優雅停止並用 `wp68_kiln_keep.sh`/`wp68_vanilla_keep.sh` 保留世界重啟 → 兩個客戶端重連 → `dump after`,再用 `persist_cmp.py`(重啟前後的差)與 `persist_kv.py`(K 對 V)比。狀態包含:
背包(帶附魔/自訂名稱/束口袋/潛影盒/藥水/成書/煙火的物品)、終界箱(5 個獄髓錠,真實用箱子放進去)、經驗、效果、重生點、世界出生點、成就(2 個)、統計(10 次跳躍)、計分板+隊伍+側邊欄+boss 條+命令儲存空間+世界邊界、遊戲規則 4 個、難度、
馬(馴服+鞍+鐵馬鎧+主人)、驢(箱子+物品)、羊駝(裝飾毯+箱子)、栓在柵欄上的牛(`leash`+`home_pos`)、有交易的村民、**有床+職業方塊的村民**(`home`/`job_site` 記憶、`Brain`)、快樂恐魔(掛具)、銅傀儡(氧化進度)、狼(坐下+狼鎧+染色項圈)、硫磺方塊(身體裝備 TNT)、
載著豬的礦車與運輸礦車、展示框(地圖)、盔甲架、人偶、**玩家正在騎的船**(重連後仍在船上)、21 種方塊實體(箱子、熔爐、釀造台、漏斗、發射器、木桶、唱片機、講台、告示牌、旗幟、裝飾罐、蜂巢、命令方塊、刷怪籠、營火、潛影盒、鐘、銅箱子、信標、層架、比較器)、
**漏斗時鐘**(兩個漏斗互傳一顆石頭,比較器讀進紅石燈:重啟前後燈都在閃)。
**結果**:除了問題 13(蜜蜂的 `ticks_in_hive` 在 Kiln 因下雨飛走而空了),所有物項重啟前後、K 與 V 之間一致;`data get` 的呈現差異(記在下面「已知差異」)與移動中的生物位置、`Motion`、時間計數不比。

## 已知差異(沒修)

- **看起來是 Kiln 的、很小**:成功的伺服器端互動(銅傀儡塗蠟/斧頭除蠟)Kiln 沒送揮手動畫;銅傀儡塗蠟時事件與音效順序相反;方塊事件的重複次數不同(vanilla 開箱子送 16 次 `block_event`,Kiln 2 次,客戶端看起來一樣);
  實體剪毛/互動的音效 vanilla 用 `sound_entity`(跟著實體),Kiln 用位置音效(剪沼澤骷髏);離玩家很遠的刷怪籠 Kiln 的 `data get block` 呈現原始存檔(沒補 `SpawnCount/MinSpawnDelay/MaxSpawnDelay/SpawnPotentials`),玩家靠近後就一樣;
  `setblock` 帶 `components:{pot_decorations:…}` 的裝飾罐 Kiln 照用,vanilla 忽略;`NoAI` 的牛被栓繩拉過之後 Kiln 的 `Motion` 殘留(位置不動,只是 NBT 數字);漏斗放進層架的物品客戶端要等下一次方塊更新才看到;
  Kiln 的 `difficulty` 在重啟後沿用存檔(`KILN_DIFFICULTY` 才覆蓋),vanilla 每次啟動用 `server.properties` 的值。
- **沒做**:快樂恐魔 4 人同騎(兩個客戶端)、重錘密度/穿透/風爆的並排傷害數字(下落距離不可控)、銅工具與護甲的一擊/受擊數字、銅火把、突襲進行中的持久化。

## 效能(優先:5 ms 目標)

整段遊玩的 Kiln `kiln_sim` 統計列(每 30 秒一行,30 行):mspt 平均 0.8~2.6 ms(整體平均 1.42)、p99 2.7~8.7 ms、最大值只在 `/fill` 與一次出現 600 格以上的指令批次時到 50~210 ms(`g.console`);
沒有 `moved too quickly` 之外的警告(一次,掉下 16 格的測試)。這次沒有大量區塊載入的場景(平台+少數實體);沒有發現異常。

## 驗證(VM,`07d83eba` 起)

| 項目 | 結果 |
| --- | --- |
| `cargo test --workspace --release --no-fail-fast`(不帶 `KILN_DATAPACK`) | 180 個測試二進位 `ok`;唯一失敗是 `mobs_match_vanilla`,**且只有 9 個 `farm69_*` 場景**(見下) |
| 同上,帶 `KILN_DATAPACK=/home/test/kiln/work/generated` | 相同:180 個 `ok`,唯一失敗同上 |
| `kiln-sim --test determinism` | 6/6 通過 |
| `kiln-entity --test mob_parity`(1451 個場景) | **1451 個全部 `ok`、0 個 FAIL**(把別的 session 剛加進 `work/m6-mobs2/vectors.jsonl` 的 11 筆 `farm69_*` 向量濾掉後跑;原檔我備份為 `vectors.jsonl.wp68bak`,沒有改動) |
| `tools/parity_suites.py --skip-workspace` | 102 套件中 101 個通過;失敗的 `mob_parity` 就是上面那 9 個 `farm69_*` 場景 |
| `crowd_golden` | 4/4 通過,**不必重錄**(群眾場景只有玩家,生物的 entity data 與爆炸封包不在其中) |
| 負向檢查 `tools/wp68_neg.py` | 16 個案例(15 個問題,問題 5 分單元與模擬兩個測試)拿掉修正後對應測試全部失敗 |

`farm69_*` 是並行的 wp69(鐵傀儡農場)session 寫進共用向量檔的新場景,對應它還沒合併的修正(村民 `golem_detected_recently` 記憶的順序與 `y = 100.5625 對 100`);我的分支沒有那些修正,所以那 9 個失敗與本分支的改動無關(wp69 合併後會過)。
