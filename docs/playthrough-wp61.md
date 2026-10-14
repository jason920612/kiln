# wp61:第四次真實客戶端遊玩(+ 第二個「旁觀者」客戶端)

目的:用官方 26.3 客戶端(跑在 VM 的 Xvfb + llvmpipe 上,由 `tools/wp58/ClientCtl.java` 驅動;客戶端不在 Windows 上跑)玩 wp58~wp60 沒玩到的東西,
並且**每個動作都和 vanilla 26.3 參考伺服器並排比較**。分支 `wp61-playthrough4`。

## 環境與工具(相對 wp60 的變動)

| 項目 | 做法 |
| --- | --- |
| 同一份地形 | Kiln 直接跑在 vanilla 世界的複本(`~/wp61/world_v`)。vanilla 的地形生成與區塊生成順序有關(vanilla 對 vanilla 也不同),所以地形、地圖顏色、結構要比就得用同一批區塊檔 |
| 旁觀者客戶端 | `tools/wp58/vm_observer61.sh`:第二個真實客戶端站在旁邊,並在 Netty 管線裡掛**封包間諜**(`ClientCtl` 的 `spy` 指令)。可以逐封包比較「伺服器送給**其他玩家**的東西」:聲音、粒子、level event、實體事件、裝備、實體資料、方塊事件。這是 wp58~wp60 一直看不到的一半 |
| 驅動新增 | `menuinfo`(容器選單狀態)、`mapdump`(地圖像素)、`weather`、`spy`;`PORT` 可用 `-Dwp61.port` / `KILN_AGENT_PORT` 改(演員 25999、旁觀者 26000) |
| 啟動 | `wp61_start.sh`(Kiln)、`wp61_vanilla.sh`(vanilla 參考,**會清掉 vanilla 世界**)、`vm_client61.sh`、`vm_observer61.sh`、`build_agent_wp61.sh`;輔助函式 `p61.py`(`both`:同一件事在兩個伺服器各做一次並比較;`platform`、`count_ents`、`nbt_of`) |
| 場地 | 空中平台+牆(死亡/掉落都出過事,見下) |

**自己踩到的坑(非 Kiln 問題)**:`pkill -f` 會殺掉自己的 ssh shell;舊的 console fifo 會卡住啟動腳本;腳本裡的 TNT 離玩家太近、
平台邊緣掉下去都會讓玩家死掉,之後的「比較」其實是在比死人。所有腳本開頭先檢查死亡畫面/`alive()`。
第一批圍城(VillageSiege)的比較是無效的(`spawn_mobs false`、沒有每次重設白天),重做後才算數。

## 玩了什麼、結果

每一項都是「在兩個伺服器各做一次,取事實(`*_facts_K/V.json`)比對」。

- **驢/騾/羊駝**:餵食、繁殖、屬性(血量、跳躍、速度)、羊駝力量與商隊(拴繩串連,`caravan`)、箱子欄位。事實一致。
- **拴繩與拴繩結(圍欄)**:綁、解、拉斷、多隻同結、存檔重讀、距離拉回。**發現並修正**:拴繩結不該自己存 `block_pos`(見下)。
- **殭屍村民治療與交易折扣**:黃金蘋果+虛弱、治療時間、折扣精確價格、八卦(gossip)、村莊英雄。**發現並修正**:交易八卦的時機(見下)。
- **地圖顏色**:兩個伺服器(同區塊檔)的像素資料、水深陰影、混合群系;一致(截圖 `docs/wp61/map_K_0.jpg`、`map_V_0.jpg`)。
- **蜜蜂**:授粉、作物生長、花、回巢。一致。
- **日常生存**:附魔台書架數(15 本,`enchant15.jpg`)、鐵砧成本/太昂貴、砂輪經驗、堆肥桶、鍋爐(`cauldron.jpg`)、睡覺跳過天氣、
  失眠後的幻翼(`phantom.jpg`)、雪與冰、三叉戟引雷(`channeling.jpg`)、書與簽名、告示牌編輯、熔爐/工具耐久。
- **村莊圍城**:時間 18000 的 1/10 擲骰、daylight 重設、32 格環、要 `isVillage`;重做後行為一致。
- **死亡/重生、傷害類型、藥水效果**:傷害與效果聲音一致。

## 發現的問題與修正(全部有測試)

旁觀者客戶端比對找出約 18 個真實差異:

1. 拴繩結存了自己的 `block_pos`(vanilla 不存,位置由實體座標決定)——移除 `save()`,`load` 忽略;`tests/leashes.rs` 驗證。
2. 村民交易八卦:vanilla 在 `customServerAiStep` 用 `lastTradedPlayer` **每 tick 一次**處理 TRADE 八卦與特價更新(NoAI 村民不會);
   Kiln 在交易當下就加。改成 vanilla 順序;升級時加再生效果 200 tick;結束交易不重設特價。
3. 交易 `Offers` NBT 只寫非預設欄位(`optionalFieldOf`)。
4. 重生點存檔:有 yaw/pitch,`forced` 只在 true 時寫(`/spawnpoint` 為 forced)。
5. 挖掘:`ServerPlayerGameMode.tick` 每 tick 送 level event 2019/2020(裂痕粒子與敲擊聲,對所有人包含挖掘者);`CHANGE_DESTROY_DIRECTION` 動作改變方向。
6. 方塊使用成功會對旁人揮手(`interact animation`);投擲物也會揮手;揮手狀態從 0 開始(客戶端每 tick 揮拳曾被每 tick 廣播)。
7. 按鈕/壓力板放開的喀噠聲;紅石開關的門、活板門、柵欄門的聲音(含鐵門類)。
8. 吃完食物的完成聲(中立類)與打嗝(玩家類),吃的人自己也聽得到。
9. 雞蛋投擲聲音類別為玩家;經驗瓶破裂 level event 1053;煙火發射聲屬於 ambient。
10. 打火石/火焰彈:點燃 TNT 方塊立即引燃(耐久/消耗與使用統計),點燃苦力怕(聲音、耐久)。
11. 風彈爆炸有自己的陣風粒子與聲音(`ExplodePacket` 帶粒子與音效),不再多送一個獨立音效。
12. F 鍵換手:`SWAP_ITEM_WITH_OFFHAND` 動作、換手實體事件 55(兩手交換物品時)。
13. 擠奶的聲音讓旁邊的人也聽到;`/give` 發出撿拾聲(音量 0.2、音調依 vanilla 公式)。
14. 玩家上線、`/op`、`/deop`、重生後,客戶端要收到權限等級實體事件 24~28(`PlayerList.sendPlayerPermissionLevel`)——Kiln 原本完全不送,
    客戶端因此永遠認為自己是 0 級(影響創造模式的操作員物品、指令方塊畫面等)。已補上,`tests/weather.rs::the_client_is_told_its_permission_level`。

## 還沒做 / 已知差異

- 其他玩家的血量實體資料(id 9)Kiln 沒送給旁觀者;全量重送玩家 metadata(vanilla 只送變動的)。客戶端畫面上看不出來(只影響其他玩家的血量讀值),先記錄。
- 死亡重生後 vanilla 另外再送一次 `CommandsPacket`/`PlayerAbilitiesPacket`;Kiln 的指令樹只在加入/權限變動時送(客戶端行為相同,未驗證邊角)。
- 地形生成本身仍不比對(vanilla 自己都因順序而異)。
- 驢/騾繁殖統計與羊駝商隊只比對了事實;沒有像 wp60 那樣大量統計抽樣。
- 客戶端 JVM 偶發崩潰(已知,重試即可,見記憶中的開發環境筆記)。

## 截圖

`docs/wp61/`:`caravan1`(羊駝商隊)、`cured_offers`(治療後交易折扣)、`enchant15`(15 書架)、`leash_2knots`/`leash_after_restart`(拴繩結存檔重讀)、
`map_K_0`/`map_V_0`(地圖並排)、`phantom`、`channeling`、`villager_levelup`、`feed_llama`、`cauldron`。
