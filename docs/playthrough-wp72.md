# wp72:第七次遊玩(技術型生存玩家的建造,並排 vanilla 26.3,同一個世界)

分支 `wp72-playthrough7`(從 main `e06dd711` 開出)。這次的教訓來自 wp69/wp71:**比較的兩邊必須是一模一樣的世界**(同種子、同產生器、同方塊、同時間、同 gamerule),
否則差異只是世界不同(wp71:flat 對虛空)。所以做法和前幾次不同:
**一支 Python 程式同時驅動兩個伺服器的 console(`tools/wp72/lock.py`)**,用 `tick freeze` + `tick step N` 讓兩邊逐刻同步,每一刻(或每 N 刻)把兩邊的方塊、實體、容器讀出來比。
真實客戶端(Xvfb、`tools/wp58/ClientCtl.java`)用在需要真人動作的地方(釣魚、導管截圖、看客戶端收到的封包);建造與取樣走指令。
所有建置與測試只在 VM(`~/kt wp72-playthrough7 …`、`CARGO_BUILD_JOBS` ≤ 5);Kiln 在 25589、vanilla 在 25595,不碰 wp73(25590/25597)。Mojang 的資料不在 repo。

## 工具(`tools/wp72/`,`push_tools.sh` 複製到 VM 的 `~/wp72`)

| 項目 | 做法 |
|---|---|
| `lock.py` | `both_`(兩邊同一道指令)、`ask`/`ask_both`(讀 log)、`step`(兩邊各走 N 刻;Kiln 走一步要 50 ms,vanilla 瞬間,所以每步之後要等)、`align`、`setup`、`respawn_all` |
| `p72_fuzz.py SEED N PAL` | 隨機紅石/活塞電路(調色盤 `all|pist|qc|rep|dupe`):兩邊每刻 `boxdump`(客戶端眼中的方塊)+實體普查;`min SEED PAL` 把失敗案例縮到最小。`ClientCtl` 新增 `boxdump`、`spy blocks`(依到達順序記錄方塊封包) |
| `p72_one*.py`、`p72_cmp.py` | 手工電路逐刻比對;所有容器的比較器讀數 |
| `p72_conduit.py`、`p72_lectern.py`、`p72_breed.py`、`p72_farm.py`、`p72_survey.py`、`p72_items.py`、`p72_ocean.py`、`p72_fish.py`、`p72_shots.py` | 各場景 |
| `agent/W72Agent`+`W72Hook` | vanilla 的 java agent(JDK ClassFile API):數 `Slime.checkSlimeSpawnRules` 的成功次數、`NaturalSpawner` 各關卡的判斷(box 擋掉幾次、能量預算擋掉幾次…)。每個轉換要各自一個 pass(`andThen` 串接什麼都沒做) |
| `SlimeChunks.java` | 用 vanilla 的 `WorldgenRandom.seedSlimeChunk` 算史萊姆區塊,對照 Kiln |

## 踩到的坑(不是 Kiln 的問題)

1. **Kiln 的 `tick step N` 要 N×50 ms,vanilla 瞬間**:腳本不等就送下一道指令,兩邊的步數對不上。我因此「發現」過一個假的「Kiln 早一刻」錯誤;`schedule_timing.rs`(4 個測試)證明 Kiln 的排程時序和 vanilla 一致。
2. 比較器的 `facing` 指向**輸入**那一側;`/setblock` 放比較器不會算輸出,要等鄰居更新。
3. `pkill -f 'mc-client/game'` 會殺掉自己的 ssh(樣式出現在自己的指令列),改用 `[g]ame`。
4. wp68 的 `switch.sh`/`vm_client68.sh` 拷貝過來會啟動舊的 jar;`push_tools.sh` 會刪掉 `*68*.sh`。
5. 同時開兩個客戶端、兩個伺服器、wp73 的 cargo,cranelift 編譯會被 OOM:建置前先 `stopclients.sh`,`CARGO_BUILD_JOBS` 3~4。
6. 玩家在兩邊都可能已經死亡並存成死亡狀態 → `respawn_all()`。
7. 客戶端顯示的效果剩餘時間是客戶端預測的,凍結時不能拿來比。
8. Kiln 日誌印的是翻譯鍵(`commands.kill.success.multiple[N]`),vanilla 印英文,解析要兩套。
9. 農場腳本:玩家掉在平台裡(補玻璃墊+重新 `tp`);Kiln 的視距/模擬距離預設 10、vanilla 8 → 啟動腳本設 `KILN_VIEW_DISTANCE=8 KILN_SIMULATION_DISTANCE=8`;`lock.py` 的 `PLAY` 來自 `KILN_PLAY_DIR`,**要透過 `run.sh` 執行**,直接 `python3` 會去寫 wp58 的 console 然後卡在 fifo 的 `open`(`wait_for_partner`)。
10. 控制台的 `@e` 在兩邊都跨維度。

## 先驗證的事(Kiln 與 vanilla 完全一致)

- **紅石逐刻 fuzz**:`all/pist/qc/rep`,160 多個隨機案例,每刻比客戶端眼中的方塊:**0 差異**(活塞推拉順序、半連接 quasi-connectivity、中繼器/比較器/觀察者鏈、黏液/蜜)。
- **比較器**:所有會輸出訊號的容器(箱子、漏斗、投擲器、熔爐、釀造台、讀台、唱片機、燭台蛋糕、蜂箱…)390 個案例:**0 差異**。
- **導管**:啟動條件、能量(導管能量 260 刻)、打擊週期(每 40 刻 4 點魔法傷害,`Target` 標籤)和 vanilla 逐刻相同 — 但 Kiln **原本根本沒有導管**(見問題 1)。
- **村民**:圖書館員重新洗牌(放書架→認領職業→拆掉→失去職業)的時間分布、16 對村民繁殖的時間,兩邊在雜訊內相同。
- **史萊姆區塊演算法**:三個種子與 vanilla 的 `seedSlimeChunk` 逐區塊相同(`kiln-entity/tests/slime_chunks.rs`)。
- **農場**:苦力怕平台(9 塊 14×14×4,y200)、海底神殿守衛者(K 400 / V 412)、地獄荒原生物普查(修好能量預算後)、哨站掠奪者(修好後 K 20 / V 18 每 3 夜)。
- **溺屍裝備**(`p72_ocean.py drowned`,同時召喚、各 900+300 隻):三叉戟 K 60/971=6.2%,V 46/941=4.9%;釣竿 K 32、V 31;鸚鵡螺殼 K 37、V 26。差在雜訊內(最大 1.3σ)。
- 物品物理(水道、泡泡柱、冰/藍冰/黏液/蜜/靈魂沙、岩漿/仙人掌/岩漿塊、活塞推、礦車漏斗):逐刻比 `Pos`/`Motion`,絕大多數相同;剩下的小差異見「未修」。

## 找到並修好的問題

| # | 問題 | 修正 | 測試(拿掉修正會紅) |
|---|---|---|---|
| 1 | **導管完全沒實作**(沒有框架判定、沒有導管能量、不打敵對生物) | 新 `kiln-sim/src/conduit.rs`(`BeKind::Conduit`;3×3×3 水框、42 個框架格、每 40 刻更新形狀、範圍 `size/7*16`、`Target` 的儲存與讀取、環境音效);`entities::hurt_by_block` | `kiln-sim/tests/conduit.rs` 3 個(拿掉 `conduit::tick` 呼叫:3 個全紅) |
| 2 | **活塞的方塊事件晚於它造成的方塊變化**:客戶端收到 Block Event 才自己模擬移動,先收到方塊變化會看不到被黏性活塞拉回的方塊(動畫 2 刻) | `BlockOut.events_from`;`blocks::finish` 把事件之後的變化延後送(vanilla 的 `ServerChunkCache` 在 `runBlockEvents` 之前刷新區塊變化) | `kiln-sim/tests/piston_packets.rs` |
| 3 | **自然生成忽略 `spawnDimensionsScale`**:史萊姆/岩漿怪要檢查 4 倍大的生成盒(2.08 格高);Kiln 檢查原尺寸 0.52,於是 2 格高的房間也生史萊姆。兩個真伺服器、種子 72、九個史萊姆區塊、13 層 2 格高的平地、14400 刻:Kiln 22~28 隻、vanilla 0 隻(java agent 數到 32 次 `checkSlimeSpawnRules` 成功都被盒子擋掉) | `spawner.rs` 用 `t.width*scale, t.height*scale`(`kiln_entity::spawner::spawn_dimensions_scale`) | `kiln-sim/tests/slime_rooms.rs`(需要 datapack,約 80 秒):2 格高 0 隻、3 格高 > 0 隻。修好後 3 格高:K 31 / V 26 |
| 4 | **指令回饋與 `say @s` 忽略自訂名稱**(印的是類型名) | `commands.rs` 的 `of_entity` 用 `CustomName` | `commands::display_name_tests`(拿掉:紅) |
| 5 | **掠奪者的生成規則錯**:vanilla 只有掠奪者用 `PatrollingMonster.checkPatrollingMonsterSpawnRules`(方塊光 ≤ 8,**不需要黑暗**);Kiln 用一般怪物規則(要夠暗)。哨站 6 夜只生 4 隻、vanilla 3 夜 18 隻 | `kinds/pillager.rs::check_spawn_rules` | `kiln-entity/tests/pillager_spawn_rules.rs`(拿掉:紅);修好後 K 20 / V 18 每 3 夜 |
| 6 | **生成成本(`PotentialCalculator`)沒做**:扭曲森林(末影人 charge 1.0、預算 0.12)、靈魂沙谷(0.7/0.15)的生物密度上限 | `spawner.rs`:`SpawnTable.costs`、`Spawner.charges`、`createState` 從現有實體累加、`canSpawn` 能量檢查在建立生物之前;單元測試 `potential_tests` | `kiln-sim/tests/spawn_costs.rs`(需要 datapack;拿掉:紅);荒原生物普查兩邊相同 |
| 7 | **指令/函式召喚的實體晚一刻才第一次 tick**:vanilla 的 `/summon` 當場把實體加進關卡,同一刻的實體迴圈就 tick 它;Kiln 在這一刻結束才讓它進場 | `lib.rs::step`:主控台指令、設定檔後與 `tick_global`(tick/排程函式)之後各 `materialize_spawns()` | `kiln-sim/tests/summon_tick.rs` 3 個(拿掉:物品第一刻測試紅) |
| 8 | **方塊階段產生的實體(掉落的沙/礫/鐵砧、紅石點燃的 TNT、方塊 tick 弄破方塊的掉落物)晚一刻**:vanilla 的 `entityTickList` 迭代涵蓋方塊 tick 加入的實體,同一刻就 tick;Kiln 晚一刻(vanilla 的掉落沙在生成那刻伺服器 `Pos` y=201.96、`Motion` -0.0392,Kiln 202.0/0.0;`dupe` 調色盤 30 案例中 21 個落地時間差一刻) | `entities::tick_spawns` + `Region::tick_block_spawns`:方塊階段新增的 FallingBlock/FallingStalactite/Tnt/Item 在同一刻補一次 `common_tick`+`tick` | `summon_tick.rs::a_falling_block_that_the_block_phase_made…`(拿掉 `tick_block_spawns` 呼叫:紅) |

**7、8 的副作用**:三個舊測試編碼了舊的「晚一刻」時序,已更新並註明原因:`carts.rs`(漏斗礦車前幾刻拿到的物品數上限 3 → 4,因為礦車在召喚那刻就開始 tick)、`creaking.rs`(讓心臟先 tick 一刻吃進 `creaking` 的資料)、`llamas.rs`(羊駝吐口水測試原本依賴第一刻的 RNG 流剛好在 40 刻內吐;改成只打一下、等 120 刻 — 遠程攻擊目標間隔本來就是 40 刻,被擊退後第二下打不到)。
這些改動不會改變 `mob_parity` 的重播(重播自己控制每刻)。

負向檢查(每個修正在 VM 上用一行文字編輯關掉,對應測試必須失敗)已全部做過:導管 3 個紅、自訂名稱 1 個紅、掠奪者 1 個紅、召喚物品 1 個紅、方塊階段掉落沙 1 個紅;活塞封包、史萊姆房間、生成成本是在做修正時各手動驗證過(沒有修正時紅)。

## 試了但沒有定論,或沒做的

| 項目 | 狀況 |
|---|---|
| 女巫小屋、末影人(終界)、黃金農場(地獄屋頂)、海龜蛋孵化 | 兩邊都 0 對 0(或數量太小),沒有結論;腳本在 `p72_farm.py`/`p72_ocean.py eggs` |
| 交易大廳、突襲、貓、徒步下界 | 沒有做完整的並排,只有繁殖/洗牌/傀儡的部分(wp69/wp71 已覆蓋傀儡) |
| 以物易物的分布 | 沒取樣;戰利品表本身由 `LootVectors` 逐項驗證 |
| 釣魚 | 見下方釣魚段 |
| 複製機(TNT/鐵軌/地毯複製器) | 沒有專門建;`dupe` 調色盤的 fuzz 只找到問題 8 的時序差 |

## 還沒修的(已知)

1. 容器裡的**未知物品 id**:vanilla 載入時丟掉,Kiln 保留原始條目。
2. 物品靜止時 `Motion` 的小差異:泡泡柱地板反彈、靈魂沙、仙人掌、活塞旁、藍冰/蜜上的次毫米漂移(位置在容許內,沒有造成可見差)。
3. 漏斗鏈:物品數有 ±1 的延遲(在鏈尾,稍後的取樣相同)。

## MSPT

`sim_load` 的負載沒有變;這次新增的兩段(`tick_spawns` 與召喚的 materialize)只在有召喚/方塊階段實體的刻才做事,平常為 0。Kiln 空閒時 mspt 平均 0.47~0.50 ms、p99 1.3~2.3 ms(1 位玩家、5 個區域、283 個區塊,視距 8)。

## 截圖

`docs/wp72/conduit_KV.jpg`:兩個客戶端(Kiln 與 vanilla)看同一座導管,都顯示導管能量圖示。
