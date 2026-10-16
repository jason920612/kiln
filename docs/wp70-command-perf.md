# wp70：方塊編輯指令的效能（`/fill`、`/setblock`、`/clone`、`/place`、`/fillbiome`、大量指令）

分支 `wp70-command-perf`（從 main `2f90e2c7` 開出，只推到 `vm`）。

wp66 記下的問題：`/fill`、`/setblock` 很慢，一次大 fill 會把 tick 卡住好幾百毫秒到好幾秒；指令方塊、資料包函數、插件都靠這些指令，不能讓伺服器停頓。
這一包照使用者的規則（Musk 五步）：先 profile、問每一項每格工作是不是必要，刪掉不必要的，再簡化，最後才加速。

## 量測工具（`tools/wp58/`）

| 檔案 | 做什麼 |
| --- | --- |
| `wp70_servers.sh K\|V start\|stop [binary]` | Kiln（port 25590，`~/wp70/K`）或原版 26.3（port 25597，`~/wp70/V`），每次都是新的 noise 世界（種子 70）、同一個 datapack（`~/wp70/pack/wp70`），console 走 fifo |
| `bench70.py pack` / `bench70.py run K\|V [--bin] [-k 案例] [--out]` | 兩邊跑同一串指令（`CASES`），每個指令後面接一個未知指令當結束標記，等它出來、過一秒再 `/tick query`：P99 在 100 個樣本時就是最近 100 tick 的最大值，也就是那個指令的 tick；P50 是閒置 tick。一個 kiln-bot（旁觀者）站在測試區上方，所以封包的建立與送出兩邊都算在內。每格成本 = (最差 − 閒置) / 方塊數 |
| `bench70.sh 名稱 …` | 同上，背景執行，輸出 `~/wp70/名稱.txt` |
| `prof70.sh binary 名稱 "指令" [秒]` | 開 Kiln、錄 `perf`（`CG=fp` 用 frame pointer 版本的呼叫圖；`SETUP` 先跑的指令；`ALT` 第二個指令，兩個輪流跑滿錄製時間） |
| `state70.py run binary 名稱` / `state70.py cmp A B` | **狀態等價檢查**：凍結 tick、關掉 random tick，跑 bench 的方塊指令和更多（形狀、紅石、流體、方塊實體、跨 region 的 fill），存檔後逐 chunk 比對兩個版本的方塊、生態域、方塊光、天空光、方塊實體、排程 tick、高度圖 |

frame pointer 版本：`CARGO_TARGET_DIR=target-fp RUSTFLAGS="-C force-frame-pointers=yes" cargo build --release -p kiln-server`（DWARF 回溯在這個 binary 上展不開呼叫者）。

（踩到的坑：fill 和 `tick query` 同時送進 console 會在**同一個 tick** 處理，query 量不到 fill 的那個 tick——第一次量出「fill 只要 0.6 ms」就是這樣，要等指令的結束標記出來再問；`pgrep -f 樣式` 的樣式出現在自己的 ssh 指令列上會永遠找得到自己；`prof70.sh` 的 `wait` 要等 perf 的 pid，不是 `%1`（那是 bot）。）

## 時間花在哪（改之前）

VM（12 vCPU）上的 profile，基準版 = main `2f90e2c7`：

- **32768 格 `/fill … stone`（空中，DWARF）**：一個 tick 0.46–0.53 秒（perf 開著時 4.4 秒）。
  - **光照 55% 以上**：`set_block` 每放一格就同步跑一次 `light::update_light`（5×5 chunk 視窗、減光 BFS 再增光 BFS）。空中放石頭時，每一格都把底下整條天空光柱（到地面約 80 格）清掉、再從旁邊重新擴散；接著放上面一格又把同一片區域再算一次——同一片光被重算幾千次。
  - **每格重建 `BlockEnv` 約 8%**：約 27 次遊戲規則查詢（`HashMap<String>` 的 SipHash，加上 `unwrap_or(...)` 讓 `game_rule_default` 的線性字串搜尋**每次都跑**），還有 Arc clone、玩家清單 collect、drop。
  - 每格各送一個 Block Update（`with_level_held` 每次呼叫都 `finish` → `send_changes`），`send_changes` 的去重是 `Vec::contains`（平方級）。
- **`function` 4096 個 `setblock`、`execute as @e … setblock` ×1000**：每個 setblock 自己算光（同上）；而且 `with_level_held` 每次呼叫都為區域裡**所有實體**建碰撞箱（`entity_boxes`）——1000 個盔甲座、每個 setblock 兩次呼叫 = 200 萬個 `EntityBox`。
- **`tp` ×10000**：`teleport_entity` 每次都用 `partition` 把每個區域的整個實體列表搬一遍（兩個新 Vec），還呼叫兩次 `update_membership(true)`，每次都全域掃描所有實體的「跨區域觀看者」。
- **4096 個箱子的 `/clone`**：`Host::block_entity`（clone 對**每一個**來源方塊都呼叫，沒有方塊實體也一樣）與 `load_block_entity` 每次都 `containers.store(chunk)`——BTreeMap 範圍掃描整個 x 條帶的所有容器——4096 個箱子就是平方級。
- **32768 格 clone（改了上面之後，frame pointer）**：`Chunk::column_top` 在每次空氣↔非空氣變更時從世界頂端逐格掃 384 格（6%）；形狀更新裡 `is_instance` / `block_class` 自己就佔 15–25%（每次三層間接查表再線性比對類別清單）。

## 改了什麼（五步的順序）

**質疑、刪除**（每格做、但其實不需要每格做的事）：

1. **每格的 `BlockEnv`**：一個方塊指令（`setblock`、`fill`、`clone`、`place`）裡什麼都改變不了它（規則、時間、玩家），所以每個方塊指令只建一次（`edits.rs`：`Host::begin_block_edits` / `end_block_edits`，`kiln-command` 的 `editing()` 包住每個方塊指令）。
2. **每格的光照**：方塊在指令裡只改方塊（`Blocks::set_block_unlit`），會影響光的位置記下來，**最外層的指令**（console 一行、玩家指令、函數、指令方塊、進度獎勵函數）結束時一次算完（`light::update_light_many`：所有位置一起清、一起擴散）。原版的光照引擎也是把 `checkBlock` 排隊、tick 之後（在光照執行緒）一起跑，所以指令執行中讀到的光本來就是改之前的光。結果相同：每一格最後的光都是「某個光源能到的最亮值」，跟一格一格算到的固定點一樣（單元測試 `changes_lit_together_match_changes_lit_one_by_one`：六輪隨機建築＋隨機變更，含玻璃、樹葉、水、冰、蜘蛛網、半磚、樓梯、蜂蜜、史萊姆、各種光源；整個區域逐格相等）。
   順帶修了一個原有的缺口：`increase` 會把「排進佇列後又被另一個減光清掉」的格子當成光源繼續擴散（原版 `LightEngine.propagateIncreases` 會跳過，`stored == queued` 才擴散），罕見情況下會讓光偏亮；批次時這種情況變多，所以一起修。
3. **每格的封包**：最外層指令結束時，變更按 section 送出：一個 section 一個 Section Blocks Update（單一變更時 Block Update），方塊實體資料跟在後面——原版 `ChunkHolder` 收集一個 tick 的變更、在 `broadcastChanges` 依 section 送出，這比原本更接近原版。
4. **每次方塊操作的實體碰撞箱**：`RegionLevel.bodies` 改成 `Bodies`（`Deref<Target=[EntityBox]>`），區域 tick 照舊交出建好的切片，指令路徑交一個 `OnceLock`，第一次有方塊行為要看時才建（大部分方塊指令根本不看）。
5. **讀/載入方塊實體時把整個 chunk 的容器寫回**：只寫回讀到的那一個（`Containers::store_at`、`Gateways::store_at`）；沒有方塊實體的位置什麼都不做。
6. **`tp` 重建實體列表、全域掃描觀看者**：只有一個實體（沒有騎乘）且留在同一個區域時就地移動（結果跟「拿出來再依 id 放回去」一樣）；其他情況用二分搜尋拿出那一疊；跨區域觀看者只在拓撲或玩家所在區域真的改變時才檢查（`update_membership(changed)`）。

**簡化**：`send_changes` 的去重改成每個 section 一個 4096 位元的集合；`column_top` 整個空 section 一次跳過；批次光照不排序、只依 8×8 chunk 群組分組。

**加速**：`is_instance` / `block_class` / `implements` 改成每個 state 一次查表（每個 state 的類別條目索引、自己的類別，加上每個條目的類別位元集）——這是全伺服器方塊行為都會用到的；遊戲規則的預設值只在規則沒被設定時才查。

改完之後（frame pointer profile，32768 格 fill 空氣↔石頭交替）：`place_block` 51%（主要是形狀更新：新方塊問六個鄰居、六個鄰居重算自己的形狀——原版做一樣的事）、批次光照約 16%、指令最後的 `update_neighbours` 約 10%、支撐檢查（`pop_off`）約 10%。

## 數字（Kiln 基準版 / Kiln 新版 / 原版 26.3）

同一個世界（noise，種子 70）、同一串指令、同一個旁觀者 bot；每個數字是兩輪的平均，三者**交錯**跑（基準、新、原版、基準、新、原版），因為 VM 上另一個 session 讓負載在 1.7–3.5 之間。「最差」是那個指令的 tick（ms），「每格」是（最差 − 閒置）÷ 方塊數（µs）。

| 指令 | 方塊數 | 基準 最差 ms | 基準 µs/格 | 新版 最差 ms | 新版 µs/格 | 原版 最差 ms | 原版 µs/格 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| fill4096 air->stone | 4096 | 91.5 | 22.24 | 9.9 | 2.31 | 19.6 | 4.26 |
| fill4096 stone->air | 4096 | 41.4 | 10.00 | 8.4 | 1.95 | 17.8 | 3.86 |
| fill32768 air->stone | 32768 | 527.2 | 16.08 | 59.9 | 1.82 | 59.1 | 1.75 |
| fill32768 replace filter stone->glass | 32768 | 229.4 | 6.99 | 56.3 | 1.71 | 54.5 | 1.60 |
| fill32768 hollow | 32768 | 699.5 | 21.33 | 56.8 | 1.72 | 53.2 | 1.57 |
| fill32768 outline | 5768 | 25.1 | 4.28 | 12.1 | 2.05 | 12.6 | 1.88 |
| fill32768 strict ->air | 32768 | 131.8 | 4.01 | 18.4 | 0.55 | 36.6 | 1.06 |
| fill32768 keep | 32768 | 155.4 | 4.73 | 59.5 | 1.80 | 38.0 | 1.10 |
| fill32768 destroy glass | 32768 | 163.2 | 4.97 | 57.1 | 1.73 | 168.4 | 5.09 |
| fill32768 water | 32768 | 783.7 | 23.91 | 122.3 | 3.72 | 96.8 | 2.90 |
| fill32768 water->air | 32768 | 324.2 | 9.88 | 52.2 | 1.58 | 62.5 | 1.85 |
| fill32768 terrain->air | 32768 | 314.5 | 9.59 | 51.0 | 1.54 | 53.3 | 1.57 |
| fill32768 terrain air->stone | 32768 | 242.4 | 7.38 | 64.5 | 1.95 | 51.0 | 1.49 |
| setblock 1 | 1 | 3.9 | - | 2.5 | - | 7.8 | - |
| setblock destroy | 1 | 1.4 | - | 1.4 | - | 5.0 | - |
| clone32768 setup | 32768 | 293.1 | 8.93 | 62.4 | 1.89 | 69.2 | 2.03 |
| clone32768 | 32768 | 643.8 | 19.63 | 60.0 | 1.81 | 94.5 | 2.81 |
| clone32768 masked | 32768 | 630.0 | 19.21 | 50.3 | 1.52 | 71.8 | 2.11 |
| clone32768 move | 32768 | 804.9 | 24.54 | 89.7 | 2.72 | 124.0 | 3.70 |
| chests4096 fill | 4096 | 38.0 | 9.11 | 21.2 | 5.00 | 29.1 | 6.40 |
| clone4096 chests | 4096 | 171.1 | 41.52 | 30.0 | 7.07 | 46.7 | 10.66 |
| clone4096 chests force onto chests | 4096 | 152.4 | 36.90 | 10.2 | 2.21 | 26.0 | 5.70 |
| fill4096 chests->air | 4096 | 38.6 | 9.20 | 14.6 | 3.36 | 15.8 | 3.19 |
| fillbiome32768 | 32768 | 7.0 | 0.19 | 5.5 | 0.15 | 18.1 | 0.47 |
| fillbiome32768 back | 32768 | 7.7 | 0.22 | 5.1 | 0.14 | 14.1 | 0.34 |
| place feature oak | 1 | 18.6 | - | 13.6 | - | 14.4 | - |
| place template | 1 | 19.8 | - | 5.2 | - | 9.6 | - |
| place structure ruined_portal | 1 | 21.3 | - | 12.1 | - | 14.8 | - |
| function setblock4096 stone | 4096 | 73.7 | 17.85 | 15.1 | 3.53 | 38.6 | 8.87 |
| function setblock4096 air | 4096 | 31.5 | 7.53 | 14.2 | 3.30 | 32.6 | 7.41 |
| function summon1000 | 1000 | 10.9 | 10.30 | 8.4 | 7.80 | 58.5 | 55.85 |
| execute as @e tp x1000 | 1000 | 67.8 | 65.85 | 9.3 | 7.30 | 77.0 | 71.90 |
| execute as @e data merge x1000 | 1000 | 16.5 | 14.35 | 13.2 | 11.30 | 105.7 | 100.60 |
| execute as @e setblock x1000 | 1000 | 638.5 | 636.25 | 40.9 | 38.70 | 42.0 | 36.60 |
| function tp10000 | 10000 | 625.0 | 62.28 | 27.4 | 2.52 | 50.2 | 4.55 |
| kill x1000 | 1000 | 9.4 | 7.00 | 6.8 | 4.60 | 21.1 | 16.45 |

- **每格成本**：新版每一種都在原版的 2 倍以內，大多數比原版快（原版的 tick 時間不含光照，原版在另一個執行緒算光；Kiln 的光照仍在 tick 裡）。最接近的是 `fill … keep`（1.80 vs 1.10 µs）、`fill … water`（3.72 vs 2.90）和地形裡的 `fill … stone`（1.95 vs 1.49）。
- **最差 tick**：32768 格（最大）fill：新版 60 ms、原版 59 ms（基準版 527 ms）；水 122 vs 97；沒有一個超過「原版自己的時間 + 50 ms」。
- **大量指令**：函數裡 4096 個 setblock 15 ms（原版 33–39，基準 32–74）；`execute as @e … setblock` ×1000 41 ms（原版 42，基準 639）；`tp` ×10000 27 ms（原版 50，基準 625）；`summon` / `data merge` / `kill` ×1000 都比原版快很多（原版每個都印一行 console）。
- **記憶體**：Kiln 新版 RSS 約 360 MB（基準約 350），原版 JVM 約 1000 MB；一次 32768 格 fill 讓 RSS 多 0–7 MB（批次的位置清單，指令結束就釋放）。

## 行為（必須跟原版一樣）

- **狀態等價**（`state70.py`）：基準版與新版各跑 54 個方塊指令（bench 的方塊指令全部，加上柵欄／牆的形狀、紅石線＋紅石塊、水、岩漿、沙、告示牌與箱子的資料、clone 方塊實體、destroy、glowstone、sea lantern、跨兩個 region 的 fill），存檔逐 chunk 比對 165 個 chunk 的方塊、生態域、方塊光、天空光、方塊實體、排程的方塊與流體 tick、高度圖：**0 差異**（每一版都比過）。第一次比對抓到的差異是地下自然岩漿在凍結前流動的時間差，改成先 `tick freeze` 再載入區域之後就是 0——也證明比對真的會抓到差異。
- **封包**：客戶端最後的方塊完全一樣，但現在是一個指令的變更合併成 section 更新。`blocks.rs` 的 `powering_by_command_reaches_the_client` 原本寫的是 Kiln 當時的行為（拉桿單獨一個 Block Update），改成原版的：同一個 section 的 10 格一個 Section Blocks Update。crowd_golden 的狀態雜湊與封包摘要**都沒變**（不用重錄：那些情境沒有方塊指令）。
- **command_diff**：**1956/1956**（基準版 1954：多出的兩行一行是時間相關的血量回復；另一行是本包順帶修的——同一個箱子、同樣的名字 `setblock` 兩次，原版第二次是「Could not set the block」：`BlockInput.place` 載入資料，再比較載入前後 `saveWithoutMetadata` 是否不同；Kiln 拿「預設值＋給的欄位」去比，現在改成套用後比較 chunk 裡的存檔副本）。另外有一次完整執行時 `execute on controller` 的 3 行不符，重跑後相符、基準版的完整執行也相符——是有 AI 的馬的時間問題，不是這一包。
- **新測試**（`crates/kiln-sim/tests/block_commands.rs`）：fill 的變更每個 section 只送一次、客戶端重建出的方塊與伺服器相同；三個 fill 一起算的光 = 一格一格 setblock 的光（整個區域逐格）；hollow／outline／keep／replace 過濾器／destroy 的結果與計數，strict 的沙子不掉、一般的會掉；clone 帶箱子內容、clone move 留下空氣；一個 `execute` 指令裡的 setblock 光照在指令結束時就好了。

- **最終驗證**（最終程式碼）：workspace 測試不帶 datapack 1327 通過 / 0 失敗；帶 `KILN_DATAPACK` 1325 通過、2 失敗——都是 `kiln-plugin-host` 的 `examples`（當時另一個 session 也在跑整套測試，負載 6.3），單獨重跑 16/16 通過（已知在高負載下不穩）；determinism 6/6、crowd_golden 4/4（雜湊不變）、mob_parity（`mobs_match_vanilla`，含 1462 個 m6 情境）通過；`tools/parity_suites.py --skip-workspace` **102/102 個 suite 全部 exit 0**；command_diff 1956/1956；最終版 vs 基準版的狀態比對只有 1 個差異：一個 trial chamber 的 vault 的 `server_data.state_updating_resumes_at` 23 vs 24——vault 第一次 tick 時記下「遊戲時間 + 20」，負載高時凍結前多跑了一個 tick，跟指令無關（之前兩次新版比對都是 0）。

## 還沒做的

- **光照仍在 tick 執行緒上**：原版在光照執行緒算，不計入 tick。最大的 fill 裡光照約佔 15%；蓋住大片天空的指令（例如在高空鋪大平台）光照的比例更高。要搬到別的執行緒就得讓光照資料可以併發讀寫，或像原版一樣讓光照晚幾個 tick 才好（要用次數而不是時間，維持可重現）。
- **封包按「最外層指令」合併，不是按 tick**：同一個 tick 裡兩行 console 指令各自送；原版合成一個。
- **`forceload add` 同步生成 chunk**：bench 開始時 4 個 forceload 區域（約 500 個 chunk）讓 Kiln 有一個 **10 秒**的 tick（原版沒有）。不在這一包的範圍，記下來。
- **`Containers::dispensers()` 每個區域 tick 都掃全部容器**：8000 個箱子時約佔閒置 tick 的 10%（0.1 ms 級）。
- `fill … water` 之後幾個 tick 的流體擴散、大量實體的區域 tick 不在這一包。

## 怎麼重跑

```
# VM 上（分支推到 vm 之後）
~/kt wp70-command-perf cargo build --release -p kiln-server -p kiln-bot
cd ~/wt/wp70-command-perf/tools/wp58
python3 bench70.py pack                                   # datapack（函數）寫到 ~/wp70/pack
python3 bench70.py run K --bin <kiln> --out k.json        # Kiln
python3 bench70.py run V --out v.json                     # 原版
python3 table70.py base=a.json new=k.json V=v.json        # 表格
python3 state70.py run <舊 kiln> a; python3 state70.py run <新 kiln> b; python3 state70.py cmp a b
CG=fp ALT="fill 0 150 0 31 181 31 air" ./prof70.sh <frame pointer kiln> p_fill "fill 0 150 0 31 181 31 stone" 14
```
