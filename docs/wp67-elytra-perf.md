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

PLACEHOLDER_BREAKDOWN

## 改了什麼

PLACEHOLDER_CHANGES

## 數字(改之前 / 改之後)

PLACEHOLDER_NUMBERS

## 還沒達到的、原因、需要什麼

PLACEHOLDER_REMAINING

## 驗證

PLACEHOLDER_VERIFY
