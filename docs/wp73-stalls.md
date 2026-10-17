# wp73:剩下的長 tick 卡頓(forceload、卸載時的存檔編碼、容器掃描、生成管線)

分支 `wp73-stalls`(從 main `e06dd711` 開出,只推到 `vm`)。

wp67 與 wp70 留下的長 tick:

1. `/forceload add` 約 500 個 chunk(wp70 bench 開頭):Kiln 一個約 10 秒的 tick(生成在 tick 執行緒上一個一個做);wp70 記為「原版沒有」。
2. 一個鞘翅飛行者偶爾 24–25 ms 的 tick:卸載的 chunk 的實體在 tick 執行緒上寫成 NBT 並壓縮(14–18 ms;很多玩家同時離開時 73 ms)。
3. 每個 region tick 都掃全部容器找發射器(8000 個箱子時約佔閒置 tick 的 10%)。
4. (有時間的話)生成管線的鎖競爭:8 個全速飛行者時約 130 chunk/s,生成執行緒看起來很忙,整個伺服器卻只用約 5.2 核。

做法照使用者的規則(馬斯克五步):先問每一塊工作是否必要、是否一定要在 tick 執行緒上;能刪就刪;再簡化;最後才加速。
所有建置、測試、量測都在 VM(12 vCPU、10 GB)上。開發期間 VM 和 wp72(真實客戶端 playthrough,Xvfb + 兩個伺服器)共用,
負載 8–12,所以開發中的比較都是交錯 A/B(同一段時間、輪流跑);報告的數字見「數字」一節(量測時 VM 的狀態寫在那裡)。

## 量測工具(`tools/wp73/`)

| 檔案 | 做什麼 |
| --- | --- |
| `servers73.sh K\|V start\|stop [binary]` | Kiln(port 25590,`~/wp73/K`)或原版 26.3(port 25597,`~/wp73/V`),每次新的 noise 世界(種子 70,和 wp70 相同)、視野與模擬距離 10,console 走 fifo |
| `fl73.py K\|V [--bin]` | forceload 測試:伺服器一起來就送 wp70 的 4 個 forceload、兩個 256 chunk 的、一個超過上限的、一個已經載入的,再 `forceload remove`、等 40 秒(卸載並存檔)、再 `forceload add`(從儲存讀回)。每個指令後面在**同一個 tick** 接一個 `setblock` 到區域最遠的角落(證明指令回來時 chunk 已載入),再接一個未知指令當結束標記;等標記出來後 `/tick query`:P99(100 個樣本)= 最近 100 tick 的最大值 = 那個指令的 tick |
| `fly73.sh BINDIR TAG ow1\|nether1\|ow8\|surv50` | wp67 的 `survival_bench` 飛行者/生存基準(port 25590);`SLOW=0` 時每個 tick 都印出各部分 |
| `parts73.py LOG` | 從每 tick 的那一行算出各部分(`ch.unload_*`、`ue.*`、`uc.*`)的分布 |
| `prof73.sh` | 跑 `fly73.sh` 的同時對 tick 執行緒(`sim`)錄 `perf` |
| `thr73.py` | 每秒取樣伺服器各執行緒(依名稱)的 CPU |
| `chest73.py` | 8000 個箱子時的閒置 tick(`/tick query` 的 P50) |
| `final73.sh BASE NEW` | 最後的量測:forceload(Kiln 基準、Kiln 新、原版)×2、1 人主世界與下界 ×2、8 人、生存 50 人、8000 個箱子 ×2,交錯 |
| `cpu73.sh BASE NEW CASE N` / `cpuan73.py` | 飛行的基準/新版成對跑,同時取樣伺服器 tick 執行緒自己的 CPU 時間(在忙碌的機器上比牆鐘 MSPT 可靠) |
| `verify73.sh` | 驗證(workspace 測試兩種模式、parity suites、command_diff) |

伺服器多了幾個計時(沒開 `KILN_PHASE_DETAIL` 時也會出現在慢 tick 的那一行):卸載時的 `ue.scan`/`ue.save_in`/`ue.leave`/`ue.store`(實體)與
`uc.blocks`/`uc.poi`/`uc.save`(方塊機制、興趣點、chunk 存檔);chunk 統計那一行多了生成管線的執行緒時間 `gen_terrain_ms`、`gen_decorate_ms`、`gen_wait_ms`。
`kiln-worldgen` 的 `fullbench` 範例會印出管線的執行緒時間與它做出的所有 chunk 的雜湊(比較兩個版本是否做出一樣的 chunk)。

## 時間花在哪裡(改之前)

開發中的量測(VM 與 wp72 共用,負載 8–12;交錯 A/B)。

### 1. `/forceload add`

**原版做的事**(`javap`):`ForceLoadCommand.changeForceLoad` 先檢查座標(±30,000,000)與數量(超過 256 個 chunk 就是 `commands.forceload.toobig`),
再對每個 chunk 呼叫 `ServerLevel.setChunkForced(x, z, true)`:`ServerChunkCache.updateChunkForced` 加 FORCED ticket,改變了就 `getChunk(x, z)`
→ `getChunkFutureMainThread`(加 ticket、`runDistanceManagerUpdates`)→ `MainThreadExecutor.managedBlock(future::isDone)`:主執行緒**一個一個**等每個 chunk
變成 FULL,等的時候只執行主執行緒的工作佇列,生成在 worker 執行緒(`Util.backgroundExecutor`,核心數 − 1)上做。回饋在全部做完之後送(`CommandResponseTracker`)。
所以**原版的 tick 也會卡住**:同一個世界(種子 70)、同一串指令,原版 26.3 的那個 tick:

| 指令 | chunk 數 | 原版 26.3 的 tick |
| --- | --- | --- |
| `forceload add 0 0 111 95` | 42 | 3.4–4.3 秒 |
| `forceload add 96 96 143 127` | 6 | 0.17–0.21 秒 |
| `forceload add 192 192 239 239` | 9 | 1.0–1.2 秒 |
| `forceload add 288 288 447 447` | 100 | 2.9–3.3 秒 |
| `forceload add 2000 2000 2255 2255` | 256 | 5.3–5.9 秒 |

wp70 寫的「原版沒有」是量錯了:它在 forceload 之後很久才開始 `/tick query`,那個 tick 早就離開 100 個樣本的窗口。

**Kiln(基準)**:`set_forced` 對每個 chunk 呼叫 `Dim::load_chunk` → `ChunkProvider::load_or_generate`:在 **tick 執行緒上一個一個生成**,
生成執行緒閒著;每個 chunk 後面還跑一次整個伺服器的 `apply_topology`。Kiln 的生成有固定的裝飾順序(MC-55596 的修正,`kiln_worldgen::order`):
一個 chunk 要 FULL,它的 3×3 都要裝飾完,每個裝飾又要先做完距離 2 以內排名較低的鄰居——單獨一個 chunk 平均要 31 個裝飾、約 60 個地形,
全部在一條執行緒上做。所以 9 個 chunk 也要 2 秒,256 個要 11–15 秒。

### 2. 生成管線(第 4 項,也是 forceload 的瓶頸)

把 forceload 改成在生成執行緒上並行之後,256 個 chunk 仍要 7–8.5 秒,`thr73.py` 顯示 9 條生成執行緒只用了 1.5–3.5 核。
把生成執行緒在 tick 等它們時改成一般優先權(本來是 `SCHED_IDLE`)完全沒有差別——不是搶不到 CPU,是管線本身在等。
`fullbench`(16×16 個 FULL chunk、一個新的管線,執行緒從共用計數器依序拿 chunk,也就是 forceload 的情形)加上執行緒時間:

| 舊管線 | 牆鐘 | 地形 | 裝飾 | **等其他執行緒** | 其他 |
| --- | --- | --- | --- | --- | --- |
| 1 條執行緒 | 14.4 秒(17.7 chunk/s) | 12.0 秒 | 2.4 秒 | 0 | 0.03 秒 |
| 9 條執行緒 | 6.9 秒(37.4 chunk/s) | 15.6 秒 | 1.2 秒 | **43.1 秒(70%)** | 1.8 秒 |

原因:`Pipeline::full` 遞迴地 `decorate` 前驅鏈(深度優先),每條執行緒都從自己要的 chunk 往同一串前驅走下去;前驅的視窗被別的執行緒借走、
或地形還在別人手上做時,它就睡在條件變數上(只會幫忙做「預取」清單裡的地形)。相鄰的請求(視野邊緣的一排、forceload 的方塊)
讓 9 條執行緒都卡在同一串前驅上。單一全域鎖本身的臨界區很短,不是問題。

### 3. 卸載時的存檔

每 20 tick 一批的卸載(`maintain_chunks`)在 tick 執行緒上:`ch.unload_chunks`(方塊機制把排程 tick、容器等寫回 chunk,興趣點寫成 NBT 並**壓縮**,
chunk 交給存檔編碼執行緒)與 `ch.unload_entities`(把離開的實體做成 NBT → **寫成位元組並 zlib 壓縮**,把實體移出模擬)。

- 1 個飛行者(主世界,`SLOW=0` 記每個 tick,約 160 批):實體部分每批 p50 0.43 ms、p90 0.77、最大 3.3 ms,其中編碼+壓縮(`ue.store`)p50 0.32、最大 3.2 ms;
  興趣點偶爾一批 6.7 ms(`uc.poi`)。
- **很多玩家同時離開**(`survival_bench` 50 人結束時一起斷線,約一萬個 chunk 一批):**一個 162.7 ms 的 tick**——`ue.store` **128.8 ms**、
  `ue.save_in` 5.6、`ue.leave` 6.9、`ch.unload_chunks` 20.0 ms。生存 50 人進行中也有 11–21 ms 的 `ch.unload_entities`。

### 4. 容器掃描

`Containers::dispensers()` 每個 region tick 被呼叫兩次(`tick_blocks` 與 `tick_block_entities` 的實體碰撞箱),每次走過 region 的**全部**容器;
同一個 tick 裡 `container::tick_block_entities` 為了找會 tick 的方塊實體(漏斗、熔爐、釀造台……)、`bell::tick` 為了找鐘,又各掃一次全部容器。
8000 個箱子(強制載入的 4 個 chunk 裡 20×20×20 的箱子)時,閒置 tick 的 P50 從沒有箱子的 0.15–0.20 ms 變成 **0.80 ms**:四次掃描約 0.65 ms。

## 改了什麼(五步的順序)

**質疑、刪除:**

1. **forceload 的生成不在 tick 執行緒上做。** `setChunkForced` 的兩半分開:`set_chunk_forced` 只記下 ticket 與「這個指令要等的 chunk」;
   指令最後呼叫 `Host::load_forced_chunks`(`Sim::load_forced_now`),一次把這些 chunk 全部當成急件交給生成執行緒(從儲存讀得到的直接讀),
   tick 等它們**並排**做完、一做好就放進 region,接著一次 `apply_topology`,再把還沒點光的照完邊界光(`Dim::load_now`、`Dim::light_now`)。
   觀察得到的行為和原版一樣:指令回來時每個 chunk 都在、可以編輯、光照完成;回饋文字與 256 的上限不變(`fl73.py` 兩邊逐行相同)。
   等 chunk 進 region 的那些標成急件,不受每 tick 安裝數量的限制(第一版漏了這點:從儲存讀回的 chunk 超過 32 個時,最遠的角落在指令回來時還沒進 region,
   `fl73.py` 的「reload」案例抓到,已修並加了測試)。
2. **刪掉:生成管線裡「等」的大部分。** `Pipeline::full` 改成:先算出目標需要的裝飾集合(`order::decoration_order`,含前驅),
   之後每次在鎖內挑**任何一個現在就能做的**(前驅都裝飾完、3×3 視窗的地形都在而且沒人借走),或是這些視窗裡還沒有的地形;都沒有時才幫忙做預取的地形或等待。
   挑到的裝飾在同一次持鎖裡借走視窗,所以不會兩條執行緒做同一個。裝飾仍然只在前驅完成後才做、視窗仍然獨佔,所以結果與順序無關(Kiln 的固定順序不變):
   `fullbench` 的雜湊在新舊管線、1/4/9 條執行緒都相同(`290fa99b4f6e6c88`、`0b1c28abfd1a8e90`、`2a6945a9c11655bf`),`tests/pipeline.rs`(`KILN_PARITY=1`)通過。
3. **刪掉:每個 region tick 掃全部容器。** `Containers` 多了一個索引 `index: BTreeMap<BlockPos, BeKind>`,只放 region tick 會找的種類
   (發射器、投擲器、鐘、會 tick 的方塊實體);`map` 改成私有,只能經 `insert`/`remove` 進出,索引跟著維護(`split_into`/`merge` 也是)。
   四個掃描改查索引,順序仍是位置順序(和原本 BTreeMap 的順序相同)。
4. **刪掉:卸載的二次方。** `store_entities` 對每個要寫的 chunk 用 `unloaded.contains(&c)`(線性搜尋)判斷是不是剛卸載的——一萬個 chunk 時是一億次比較(27 ms);
   改成集合。`RegionBlocks::chunk_unloaded` 每個 chunk 對整份「生成殘留」「活塞」「蓋子」「第一批動物」清單做一次 `retain`;改成每批一次(`chunks_unloaded`)。

**移出 tick:**

5. **實體與興趣點的編碼、壓縮在自己的執行緒上做**(`kiln-storage/src/encoder.rs`,`EntityStore` 與 `PoiStore` 共用)。
   tick 只把 chunk 的 compound(`Arc<Tag>`)交出去就繼續;一條常駐的執行緒依序編碼(region 檔壓縮,native 世界只寫 NBT),做完的下次 tick 有用到時收回。
   設計成**不依賴當下有沒有空的核心**(wp67 用臨時執行緒並行,在客戶端佔著 CPU 時反而更糟):
   - **讀回永遠是最新的、不等執行緒**:交出去還沒回來的 chunk,`load` 直接從交出去的那份 compound 讀(它就是最新的資料);
     回來的結果只有「還是那個 chunk 最新的一份」(序號相同)才收,舊的、或之後又存了空的(刪除)的,一律丟掉——不會讀到舊的、也不會重複。
   - **存檔不等它**:`flush`(自動存檔、`save-all`、關機)把還在外面的全部在呼叫的執行緒上自己編碼(執行緒還沒拿的從它的佇列拿走;正在做的那一個再做一次,結果丟掉),
     所以存檔的內容與以前完全一樣,也不會因為那條執行緒拿不到 CPU 而卡住。關機照舊 `save` → `sync`(寫入執行緒寫完才停)。
   - 測試:`entities_read_back_while_encoding_are_the_newest`(40 個 chunk × 30 輪,隨機存、讀、flush,最後重開檔案比對)、
     `newest_compound_while_encoding`(興趣點);把「從交出去的那份讀」拿掉兩個都會失敗。

**試過、刪掉的:**
- forceload 等待時把生成執行緒改成一般優先權——在忙碌的 VM 上交錯 A/B 兩輪,256 個 chunk 4.64/4.60 秒 vs 4.78/4.68 秒,沒有差別,刪掉。
- forceload 等待時讓 tick 執行緒自己也拿急件來生成(像原版 `managedBlock` 等待時執行工作)——交錯 A/B 兩輪沒有變快,反而 5.7/4.8、5.5/5.2 秒
  vs 3.2/2.9、5.1/4.3 秒(tick 在自己那個 chunk 做完之前不能回來,拉長了尾巴),撤回。

**沒有改的:** 卸載的時機(每 20 tick 一批)、裝飾的固定順序、生成執行緒的優先權(`SCHED_IDLE`)、區塊封包、光照時機、近似開關、模擬距離。

## 數字

**VM 一直沒有安靜下來**:wp72(兩個真實客戶端 + 兩個伺服器)整段時間都在跑,`~/wp72.done` 等了 6 小時(18:08–00:08)沒有出現,照規則之後還是量了,
所以下面的數字都是在**忙碌的 VM** 上量的(1 分鐘負載 6–16,其他程式約佔 3–5.5 核)。比較都是交錯的(基準、新、原版輪流),同一個 binary 兩次之間的差異也一併列出;
絕對值(特別是 MSPT)比 wp67 在安靜 VM 上的數字高約 2–3 ms。「基準」= main `e06dd711` 的建置,「新」= 本分支最後的建置(程式碼與驗證時相同)。

### forceload(`fl73.py`;`final73.sh` 兩輪 + 另外兩輪;同一個種子 70 的新世界;數字是那個指令的 tick,ms)

| 指令 | chunk 數 | Kiln 基準 | **Kiln 新** | 原版 26.3 |
| --- | --- | --- | --- | --- |
| `forceload add 0 0 111 95` | 42 | 1951 / 1996 | **622 / 693** / 342 / 713 | 3169 / 4344 / 4101 / 3469 |
| `forceload add 96 96 143 127` | 6 | 679 / 731 | **256 / 366** / 157 / 346 | 231 / 280 / 196 / 310 |
| `forceload add 192 192 239 239` | 9 | 2428 / 2494 | **772 / 811** / 432 / 797 | 858 / 1084 / 1214 / 1278 |
| `forceload add 288 288 447 447` | 100 | 7257 / 6810 | **2065 / 2486** / 1466 / 2367 | 2797 / 2998 / 3158 / 3409 |
| `forceload add 2000 2000 2255 2255` | 256 | 13431 / 13145 | **4312 / 4589** / 3200 / 5054 | 3542 / 6165 / 6048 / 6105 |
| `forceload add -3008 1008 -2753 1263` | 256 | 10633 / 10852 | **3680 / 4137** / 2895 / 4345 | 2510 / 4300 / 4206 / 4434 |
| 放掉、存檔後再 `forceload add`(從儲存讀) | 256 | 137 / 146 | **95 / 158** / 191 / 153 | 579 / 483 / 492 / 513 |

- Kiln 新版比基準快 3–3.5 倍。和原版比:42、9、100 個 chunk 都比原版快;256 個(第一個區域)平均 4.3 秒 vs 原版 5.5 秒,第二個區域 3.9 vs 3.9 秒;
  從儲存讀回 0.15 vs 0.52 秒。6 個 chunk 的那一個差不多(0.28 vs 0.25 秒):Kiln 的固定裝飾順序讓「一個孤立的 chunk」比原版多做很多鄰居的工作,
  數量少時看得出來,數量多時鄰居共用就看不出來。原版 256 個的那一列今天量過 6 次,從 3.5 到 6.2 秒都有(負載不同);Kiln 新版 7 次在 3.2–5.1 秒。
- 每一個案例兩邊的回饋逐行相同(`Marked 256 chunks in minecraft:overworld from [125, 125] to [140, 140] to be force loaded`、
  `Too many chunks in the specified area (maximum 256, but specified 272)`、`No chunks were marked for force loading`、`Unmarked 256 chunks …`),
  同一個 tick 裡最遠角落的 `setblock` 兩邊都是 `Changed the block at …`。
- 生成管線本身(`fullbench` 16×16):9 條執行緒 6.9 → **3.4–3.8 秒**(37 → 68–75 chunk/s),4 條 10.7 → 5.6 秒;等待從 43.1 秒降到 3.6–5.0 秒(執行緒時間的 10–14%)。

### 卸載(生存 50 人,結束時 50 人同時斷線)

| | 基準 | **新** |
| --- | --- | --- |
| 斷線那一批卸載的 tick | **188.9 ms**(`ch.unload_entities` 148.3、`ch.unload_chunks` 40.1);開發中另一次 162.7 ms(`ue.store` 128.8) | **49.0 ms**(`ch.unload_entities` 20.6、`ch.unload_chunks` 27.7) |
| 斷線那一批的細項(新) | | 實體:做成 NBT 6.1、移出模擬 9.0、交出去 4.4 ms;chunk:方塊機制 18.0、chunk 存檔 5.6、興趣點 1.9 ms |
| 進行中每批 `ch.unload_entities` p50 / p90 / p99 / 最大 | 0.99 / 3.71 / 12.54 / 148 ms | 0.86 / 1.77 / 5.46 / 20.6 ms |
| 進行中每批 `ch.unload_chunks` p99 / 最大 | 7.6 / 40.1 ms | 16.4 / 27.7 ms |

剩下的是工作量本身:一萬個 chunk 每個約 2 µs 的方塊機制、幾千個實體做成 NBT(原版的 `EntityStorage.storeEntities` 也是在主執行緒上 `entity.save`,
只有寫檔在別的執行緒);原版的 `ChunkMap.processUnloads`(`javap`)在待卸載多於 2000 個時也不看時間、一次做掉。

1 個飛行者時(每 tick 都記,`SLOW=0`,約 160 批):實體部分每批 p50 0.43 → 0.16 ms、p90 0.77 → 0.30 ms;編碼+壓縮(`ue.store`)從每批都有(p50 0.32 ms)
變成 152 批裡只剩 15 批 ≥ 0.05 ms(交出去以外的工作:實體走進沒有 region 的 chunk 時與已存的合併)。

### 容器(8000 個箱子,`chest73.py`,`/tick query` 的 P50,解析度 0.1 ms)

| | 基準 | **新** |
| --- | --- | --- |
| 8000 個箱子的閒置 tick | 0.80 / 0.81 / 0.79 / 0.90 ms | **0.13 / 0.20 / 0.20 / 0.10 ms** |
| 同一個伺服器把箱子清掉之後 | 0.15 / 0.20 / 0.10 / 0.20 ms | 0.10 / 0.15 / 0.20 / 0.10 ms |

### 飛行(`survival_bench`,wp67 的機器人,種子 4242,150 秒量測窗,真實節拍,使用者的預設設定)

| 情境 | 基準 MSPT 平均 / p99 / 最大 | 新 MSPT 平均 / p99 / 最大 | 其他 |
| --- | --- | --- | --- |
| 主世界 1 人(`final73.sh`) | 5.52 / 16.48 / 65.05;5.05 / 15.88 / 34.84 | 5.82 / 17.01 / 56.24;5.06 / 15.10 / 36.95 | 進入視野到收到 平均 / p99:667 / 2235、905 / 3763 → **355 / 1056、266 / 784 ms** |
| 主世界 1 人(`cpu73.sh`,3 對) | 5.78 / 16.82 / 31.90;5.49 / 16.46 / 40.18;5.30 / 16.17 / 24.29 | 5.85 / 15.69 / 46.27;5.77 / 18.43 / 45.09;5.37 / 15.34 / 22.17 | |
| 下界頂上 1 人(`final73.sh`) | 5.23 / 13.69 / 21.48;5.65 / 14.14 / 67.27 | 6.24 / 15.58 / 26.38;6.37 / 15.00 / 21.39 | 218 / 565、254 / 592 → 203 / 337、212 / 348 ms |
| 下界頂上 1 人(`cpu73.sh`,2 對) | 6.60 / 16.27 / 51.37;6.44 / 15.13 / 21.01 | 6.50 / 15.49 / 23.75;6.20 / 15.06 / 27.21 | |
| 主世界 8 人 | 5.34 / 13.98 / 44.10 | 5.85 / 18.29 / 84.26 | 生成 **67.2 → 88.6 chunk/s**,飛了 19,488 → **25,091** 格,停在地形邊緣 13,984 → 10,431 tick,進入視野到收到 12.5 → 9.5 秒 |
| 生存 50 人(負載 16) | 23.95 / 52.63 / 139.13 | 25.03 / 52.02 / 192.67 | 走路視野完成 p99 3.4 → 2.7 秒,加入 53.0 → 45.5 秒 |

在這台忙碌的 VM 上 MSPT 跟著「其他程式」的負載走(同一個 binary 前後兩次就差 0.5 ms 以上;`final73.sh` 的下界兩對剛好新版都碰到較高的外部負載,4.7–4.8 vs 3.6–4.1 核)。
為了把外部負載拿掉,`cpu73.sh` 同時取樣 tick 執行緒(`sim` 與 `kiln-tick` 工作者)自己的 CPU 時間:

| | 基準:每 tick 的 tick CPU(ms) | 新:每 tick 的 tick CPU(ms) |
| --- | --- | --- |
| 主世界 1 人 | 5.25 / 4.67 / 4.70(平均 4.87) | 5.11 / 5.12 / 4.68(平均 4.97) |
| 下界頂上 1 人 | 7.67 / 7.60 | 7.59 / 7.59 |

tick 自己的工作沒有變多(差異在同一版本兩次之間的範圍內;外部負載最低的那一對 4.70 vs 4.68)。新版的生成執行緒用的 CPU 比較多
(主世界 1 人 1.2–1.3 → 1.7–1.8 核,同樣數量的 chunk):以前大部分時間在等,實際上只有 1–2 條在跑;現在真的並行,在超執行緒的 VM 上每個 chunk 的 CPU 時間變長
(地形單條執行緒約 13 ms/chunk,9 條並行時約 26–29 ms/chunk)。換到的是 chunk 到得快一倍(上表的「進入視野到收到」)。

**單一飛行者偶爾 24–25 ms 的 tick**:這台 VM 上無法判斷(兩版的最大值都在 22–67 ms 之間跳,都是外部負載);每 tick 的分布裡,
卸載已經不在最慢的 tick 裡,最慢的是 region 的 `chunks`(一次做很多區塊封包)與 `blocks`(新 chunk 的樹葉等),見「還沒做的」。

## 驗證

VM 上,release,最後的程式碼(之後只有工具與文件的 commit):

- `cargo test --workspace --release --no-fail-fast`:**不帶 `KILN_DATAPACK` 1,336 個通過、0 失敗**;**帶 `KILN_DATAPACK=/home/test/kiln/work/generated` 1,336 個通過、0 失敗**。
  其中 `kiln-sim --test determinism` 6/6、`crowd_golden` 4/4(封包摘要與狀態雜湊**沒有重錄**)、plugin 決定性 1/1。
- `tools/parity_suites.py --skip-workspace`:**102 個 suite 全部 exit 0**;`mob_parity`:**1462 個情境、871,300 個 mob 狀態逐項相同**(與之前 main 的執行完全一樣,
  包括同樣兩個已知的 `DIVERGES` 行);`worldgen`(`KILN_PARITY=1`)含 `chunks_do_not_depend_on_thread_count_or_order` 與原版 chunk 比對。
- `tools/command_diff.py`(Kiln 25590、原版 25597):**1954/1956**,兩次完整執行都是同樣兩行不符:`fetchprofile id 00000000-…-0000` 與 `…-0001`
  的回答來自網路查詢,在 5 秒窗口之後才到(原版第一個的回答落到第二個窗口);單獨跑 `fetchprofile` 這一節 **43/43**。本包沒有碰設定檔查詢。
- 新測試(都在 VM 上把對應的改動拿掉確認會失敗,再還原):
  - `kiln-storage`:`entities_read_back_while_encoding_are_the_newest`、`newest_compound_while_encoding`(拿掉「從交出去的那份讀」→ 兩個都失敗)。
  - `kiln-sim/tests/forceload.rs`:`generated_forced_chunks_are_loaded_and_lit_when_the_command_returns`(拿掉立即點光 → 失敗)、
    `forced_chunks_read_from_storage_are_in_their_regions_when_the_command_returns`(拿掉急件標記 → 最遠角落「not loaded」,失敗)。
- 生成結果不變:`fullbench` 的雜湊在舊管線(`6d84a819` 加上會算雜湊的 fullbench,分支 `wp73-oldpipe-check`)與新管線相同(1、4、9 條執行緒的三個方塊)。

## 還沒做的

- **forceload 仍然是一個長 tick**(和原版一樣:指令要等 chunk 生成完)。剩下的是生成本身的成本:地形佔生成執行緒時間的 2/3(單條執行緒約 13 ms/chunk),
  加上 Kiln 的固定裝飾順序讓孤立的 chunk 要做更多鄰居。要再快需要加速地形(noise、密度函數、含水層),不是排程。
- **8 個全速飛行者仍然跟不上**(88.6 chunk/s,需要約 340):管線的等待只剩 9%,瓶頸是地形的 CPU(加上超執行緒的爭用)。
- **單一飛行者的偶發長 tick**:卸載已經移走;剩下的是一次做很多區塊封包(`chunks`)與新 chunk 的方塊 tick。需要在安靜的 VM 上重量才看得出還有多少;
  封包事先在背景做的效益不確定(新 chunk 很快又因為方塊改變而失效)。
- **大量玩家同時離開** 49 ms:剩下的是每個 chunk/實體的必要工作;原版也一次做完。要再降可以把卸載分到幾個 tick(用數量、不用時間,維持可重現),
  但會改變卸載的時機,本包沒有做。
- 同一次斷線時另一個 tick 的 `movement`(實體追蹤)在忙碌的 VM 上到 159–192 ms(wp67 也記過生存機器人的 `movement` 尖峰),不在本包範圍。
- 這些數字要在安靜的 VM 上重量一次(`final73.sh`、`cpu73.sh`)。

## 怎麼重跑

```
# VM 上(分支推到 vm 之後)
~/kt wp73-stalls cargo build --release -p kiln-server -p kiln-bot --bins --example survival_bench
cd ~/wt/wp73-stalls/tools/wp73
python3 fl73.py K --bin <kiln>; python3 fl73.py V                 # forceload,Kiln / 原版
./fly73.sh <有 kiln、kiln-bot、survival_bench 的目錄> TAG ow1      # ow1 | nether1 | ow8 | surv50;SLOW=0 每 tick 都記
python3 parts73.py ~/wp73/out/TAG-gen-anvil.server.log            # 卸載各部分的分布
python3 chest73.py --bin <kiln>                                   # 8000 個箱子
./final73.sh <基準目錄> <新目錄>; ./cpu73.sh <基準> <新> ow1 3; python3 cpuan73.py ~/wp73/cpu
KILN_WORK=~/kiln/work cargo run --release -p kiln-worldgen --example fullbench -- 16 1 9 4   # 管線,印出雜湊
```
