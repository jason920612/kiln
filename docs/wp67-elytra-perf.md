# wp67:鞘翅飛行時的 MSPT 尖峰

分支 `wp67-elytra-spikes`(從 main `8b57297f` 開出)。起點是 wp65 用真實 26.3 客戶端量到的數字:鞘翅+煙火火箭以約 32.8 格/秒直線飛行時,
主世界 MSPT 平均 2.9~4.1 ms、p99 16~22 ms、最大 25 ms;下界頂上平均 4.0~5.9 ms、p99 約 30 ms、最大 49 ms。
目標:兩個維度在這種飛行中平均 < 5 ms、p99 < 10 ms、最大 < 25 ms,而且區塊生成/傳送仍跟得上鞘翅的速度(客戶端不能比以前更常飛進沒有地形的地方)。
做法照使用者的規則(馬斯克五步):先問每一塊工作是什麼、是否一定要在 tick 執行緒上;能刪的刪;再簡化;最後才加速。
所有建置、測試、量測都在 VM(12 vCPU、10 GB)上;同時有另一個工作階段(wp66)在用 VM,所以開發期間的比較都是交錯 A/B(基準、新版、基準、新版)。

## 量測工具(新增)

- **`kiln-bot --behavior fly`**(`crates/kiln-bot/src/flight.rs`):機器人像真的玩家一樣起飛:用指令到起點(`execute in <維度> run tp`)、
  `item replace … armor.chest with elytra`、`give … firework_rocket`,往下掉一點再送 `START_FALL_FLYING`(26.x 的 6)開翼,之後每個客戶端 tick 送一個位置、
  每 2 秒用一支火箭(伺服器真的生成附在玩家身上的煙火實體),每 60 秒補火箭和鞘翅。主世界會依已收到區塊的高度圖爬升越過山(最低 y=150,離地至少 12 格),
  下界在頂上 y=180。**跟 26.3 客戶端一樣,它不會在自己還沒收到的區塊裡移動**:地形沒到時就停在邊緣等(這是「飛進沒有地形的地方」的量法)。
- 機器人量的(暖機之後):每 tick 沿飛行方向到第一個還沒收到的區塊有幾格(「前方已就緒」,上限是視野邊緣)、停在地形邊緣的 tick 數、
  每個區塊從進入視野到收到的時間、被伺服器拉回(teleport)的次數。
- **`survival_bench` 可以加飛行者**:`--flyers N --fly-dim … --fly-center x,z --fly-heading x,z --fly-spacing … --fly-altitude …`,`--count 0` 時只有飛行者;
  也可以和 wp47 的生存機器人一起跑。結果多了 MSPT 直方圖、量測窗內慢 tick(`KILN_SLOW_PRINT`)的各階段平均、飛行者的報告;Linux 上也讀伺服器與機器人的 CPU。
- 伺服器多了幾個 `KILN_PHASE_DETAIL` 計時:`e.initial_mobs`、`e.block_entities`、`c.packets`、`ch.unload_chunks/entities/forget`。

重跑(VM 上,先 `~/kt wp67-elytra-spikes cargo build --release -p kiln-server -p kiln-bot --bins --example survival_bench`):

```text
cd ~/wt/wp67-elytra-spikes
target/release/examples/survival_bench --server target/release/kiln --bot target/release/kiln-bot \
  --datapack /home/test/kiln/work/generated --world /tmp/wp67-world --phase gen \
  --count 0 --flyers 1 --fly-dim minecraft:overworld --fly-center 9000,0 --fly-heading 1,0 --fly-altitude 150 \
  --warmup 20 --measure 150 --port 25597 --slow-print 10 --out /tmp/wp67-out --tag ow1
# 下界:--fly-dim minecraft:the_nether --fly-center 2000,0 --fly-altitude 180
# 8 個飛行者:--flyers 8(側向相距 --fly-spacing 1024 格,各自一個 region)
# 加上生存機器人:--count 50(wp47 的 50 人生存基準)再加 --flyers 2
# 各階段:加 --phase-detail(有額外的計時成本;報告的數字沒有開)
```

預設是使用者的設定:tick pool = 核心數 − max(2, 核心數/4)(VM 上 9 條)、一般優先權、視野與模擬距離 10、近似開關(`KILN_ENTITY_TICKING`、locator 間隔)關。

## 尖峰的時間花在哪裡(改之前)

開發期間的量測(VM 與 wp66 共用,`--phase-detail`,1 個飛行者,150 秒量測窗,視野 10;「慢 tick」= 超過 10 ms 的 tick,數字是這些 tick 的各階段平均):

| 情境 | MSPT 平均 / p99 / 最大 | 慢 tick | 慢 tick 裡最大的幾塊 |
| --- | --- | --- | --- |
| 主世界 1 人(基準) | 6.2~6.5 / 21.6~23.9 / 31~33 ms | 590~740 / 3000 | **新區塊的光照 6.3~7.0 ms**;方塊 3.3~3.5(排程方塊 tick 1.5,幾乎都是樹葉;流體 0.7;生成殘留 0.7;隨機刻 0.3);實體 3.1~3.2(**新區塊的第一批動物約 2.5**,實體 tick 0.6);自然生怪 0.45;送區塊 0.35 |
| 下界頂上 1 人(基準) | 6.2~7.2 / 21~27 / 31~49 ms | 670~770 / 3000 | **新區塊的光照 10.6~11.5 ms**(岩漿海的方塊光);方塊 2.0~2.5(岩漿流動 1.5~2.0);實體 1.0~1.2 |
| 主世界 8 人(基準) | 11.1 / 26 / 51 ms | 1737 / 3000 | 光照 10.7(各 region 加總)、實體 9.5、方塊 7.7、生怪 3.4;偶發:B0 等存檔編碼器 40~170 ms、一個 region 一次做 64 個區塊封包 46 ms |

`perf`(tick 執行緒,主世界 1 人):最熱的單一函式是 `ParameterList::search`(生物群系的氣候查找)——新區塊的第一批動物(`NaturalSpawner.spawnMobsForChunkGeneration`)
為了讀**一個**生物群系值(區塊角落的 zoom 查找),每個區塊都把整個區塊的生物群系(1536 個氣候取樣)重新算一次;其次是光照的 BFS(每一步兩層 hash 查找與調色盤讀取)。

問「這一定要在 tick 執行緒上嗎」:
- 新區塊的光照:vanilla 在光照引擎的執行緒上做(LIGHT 狀態),不在主執行緒。區塊自己內部的光(發光方塊、天空光往下與往旁邊)不需要鄰居,可以在生成執行緒做;
  只有跨邊界的部分需要鄰居(在 region 裡)。
- 第一批動物:需要區塊已點亮(亮度規則)與方塊,留在 region;但「重新生成生物群系」完全不需要存在——區塊本身已存著同一個值。
- 方塊 tick(樹葉):vanilla 的遊戲邏輯(生成時被標記為後處理的樹葉,在區塊變成 FULL 時排 1 tick 後的距離檢查),一定要做,只能變快。
- 流體:同上,遊戲邏輯。
- B0 的等待:讀一個剛卸載、還在存檔編碼器佇列裡的區塊時,tick 執行緒會等整個佇列——這個等待不需要存在。
- 區塊封包:一定要在送出時做(要和之後的方塊更新順序一致),但不必一個一個做。

## 改了什麼

依五步的順序:

1. **刪除:第一批動物的生物群系不再重新生成。** 讀區塊已存的生物群系(與重新算出來的是同一個值:生成時存的就是這個算法的結果);只有 zoom 選到的角落落在
   還沒載入的鄰居時才算,而且每條執行緒保留最近 64 個區塊的結果(新區塊的鄰居會被下一個新區塊再問到)。`ParameterList::search` 從 tick 執行緒的第一名消失。
   (`crates/kiln-sim/src/spawner.rs`;`initial_mobs.rs` 的 vanilla 比對照樣通過。)
2. **移出 tick:新區塊自己的光在生成執行緒算。** `kiln_world::light::light_alone` 在生成執行緒上把區塊當作沒有鄰居點亮;region 收下它時
   `light_new_chunk` 只補跨邊界的部分:鄰居比較高的柱子照進來的天空光、鄰居邊上的光照進來、自己邊上比隔壁亮兩級以上的光照出去。光只會增加,
   每一格的最終值是「某個光源能到達的最亮值」,和傳播順序無關,所以結果與一次全部點亮相同。測試 `a_chunk_lit_alone_then_at_its_borders_gets_the_same_light`:
   隨機地形 5×5 區塊、隨機安裝順序,兩種做法每一格每一層都一樣(把邊界種子拿掉就失敗)。
3. **加速:光照在區塊窗口裡做。** 新區塊用 3×3、方塊改變用 5×5 的區塊窗口(區塊暫時從 cell 拿出來,做完放回):光每往旁邊一格至少少 1,
   所以不會離開窗口,每一步不再查兩層 hash;整段 0 或全滿的 section 一次跳過。測試 `block_changes_relight_within_the_window_as_over_the_whole_world`
   (隨機改方塊 120 次,窗口版與全世界版每格相同;窗口縮成 1 就失敗)。
4. **簡化:每 tick 安裝的生成區塊從「每個維度 8 個」改成「每個 region 8 個」**,新 cell 等待中的區塊也算在內。8 個飛行者各自的 region 各自吸收自己的突發
   (並行),不再互相排隊;一個 region 一個 tick 最多處理 8 個新區塊的邊界光、第一批動物與生成殘留。
5. **加速:樹葉的 tick。** `getDistanceAt` 與「設定距離」改查表(原本每次是 tag 名稱的字串雜湊加屬性名的字串比對)、六個鄰居在同一個區塊時一次查找、
   距離沒變就不呼叫 `setBlock`(vanilla 設同一個狀態也什麼都不做);排程 tick 的集合改用快速雜湊;`BlockId::by_name` 不再配置字串。
   測試 `leaf_distance_tables_match_the_properties`(所有方塊狀態)。
6. **加速:流體找下坡。** `getSpread`/`getSlopeDistance` 的遞迴搜尋同一格只讀一次(vanilla 的 `SpreadContext` 也是這樣)。
7. **刪除:tick 不再等存檔編碼器。** 要讀的區塊還在編碼器佇列裡(或它的 region 檔正在改寫)時回 `StartLoad::Busy`,下一 tick 再要;
   以前會在 B0 等整個佇列(8 個飛行者時一次 40~170 ms 的同步卡頓)。
8. **加速:區塊封包。** 一個 tick 要送給 region 裡玩家的區塊封包,先用 tick pool 的 window 一起做好(由 pool 自己決定要不要分給別的 worker),
   之後照原本的順序送;封包內容與順序不變。

沒有改的:卸載的時機(每 20 tick 一批,`crowd_golden` 與決定性的狀態雜湊依賴它)、光照更新的時機(方塊改變時立即更新,遊戲邏輯讀到的光不變)、
每個玩家每 tick 送的區塊數(客戶端要求的 `chunks_per_tick`)、模擬距離、近似開關。

## 數字(改之前 / 改之後)

PLACEHOLDER_NUMBERS

## 還沒達到的、原因、需要什麼

PLACEHOLDER_REMAINING

## 驗證

VM 上,release:

- `cargo test --workspace --release --no-fail-fast`:**不帶 `KILN_DATAPACK`** 與 **帶 `KILN_DATAPACK=/home/test/kiln/work/generated`** 都是 166 個測試目標、**1,280 個測試全綠**
  (含新的 `a_chunk_lit_alone_then_at_its_borders_gets_the_same_light`、`block_changes_relight_within_the_window_as_over_the_whole_world`、
  `leaf_distance_tables_match_the_properties`、`a_refused_glide_start_sends_the_flags_again`)。`crowd_golden` 4/4(封包摘要與狀態雜湊**沒有重錄**)、
  `kiln-sim --test determinism` 6/6。(第一次全跑時 `kiln-plugin-host --test examples` 的 `spawn_protection_denies_near_spawn…` 在 VM 負載 13 時失敗一次,
  單獨重跑 3 次都過,第二次全跑也過——已知的負載敏感測試,與本次改動無關。)
- `kiln-entity --test mob_parity`:**1451 個情境、594,683 個 mob 狀態逐項相同**(`work/m6-mobs2/vectors.jsonl`,1480 行,沒有新增)。
- `tools/parity_suites.py --skip-workspace`:**102 次執行全部 exit 0**(改動 glide 之後只重跑了 mob_parity,也通過)。
- 負向檢查(在 VM 的工作樹上暫時改壞,新測試都會失敗,之後 `git checkout` 還原):拿掉邊界光的種子、把方塊改變的光照窗口縮成 1×1、
  不重送旗標。

### 驗證時找到的 vanilla 差異(已修正)

真實客戶端在負載很高的 VM 上第一次起飛時,伺服器拒絕了 `START_FALL_FLYING`(那一刻伺服器還認為它在地上——wp65 也記過這種負載下的失敗),
**之後客戶端一直以為自己在滑翔**:落地也不會停,再按跳躍也不會再送開翼的指令,火箭用了伺服器也不認(沒有消耗、沒有加速)。
原因:vanilla 拒絕時呼叫 `stopFallFlying()`,它把滑翔旗標**先設成 true 再設回 false**(`javap` 確認),這會把旗標標成 dirty,
下一次 `sendDirtyEntityData` 就把共享旗標(不在滑翔)送給客戶端與旁觀者,修正客戶端自己先做的預測;Kiln 只在值改變時才送,所以什麼都沒送。
修正:`glide.rs` 拒絕時標記旗標要重送(`flags_resend`),`players::pack_dirty` 會把共享旗標放進下一個封包。測試 `player_sync.rs::a_refused_glide_start_sends_the_flags_again`。
