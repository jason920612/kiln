# wp74:第八次遊玩(wp72 沒做完的:農場長跑、釣魚/以物易物取樣、交易大廳、貓、突襲、徒步下界、複製機、快樂恐懼幽靈四人騎乘)

分支 `wp74-playthrough8`(從 main `d02ecd9f` 開出)。方法沿用 wp72:**比較的兩邊必須是同一個世界**(種子 72、noise 產生器、同方塊、同時間、同 gamerule),
一支 Python 同時驅動兩個伺服器的 console(`tools/wp74/lock.py`),需要真人動作的地方用真實客戶端(Xvfb :99、`tools/wp58/ClientCtl.java`:Kiln 的客戶端 agent 25999、vanilla 的 26000)。
所有建置與測試只在 VM(`~/kt wp74-playthrough8 …`,`CARGO_BUILD_JOBS=4`);Kiln 25589、vanilla 25595;工具在 `tools/wp74/`(`push_tools.sh` 複製到 VM 的 `~/wp74`)。

## 工具(`tools/wp74/`)

| 項目 | 做法 |
|---|---|
| `lock.py` | wp72 的 `lock.py` 加上 `sprint`、`kill_count`、`count`(`execute if entity` 的數量)、`locate_xz`;`setsim.sh N` 以模擬距離 N 重啟兩個伺服器 |
| `p74_farm.py` | wp72 的 `p72_farm.py` 加上 `BOX=x0,y0,z0,x1,y1,z1`(計數盒)與 `KILLBLOCK=1`:一個重複指令方塊每刻殺掉盒外所有實體(`command_block_output false`),讓怪物上限不被其他生物佔滿 |
| `p74_barter.py` | 40 隻豬布林各丟一個金錠,數掉出的戰利品 |
| `p74_fish.py` / `fish_all.sh` | 真實客戶端釣魚(兩邊同時),`tick rate 100`,開放水域/非開放水域 × 附魔 |
| `p74_village.py`、`p74_hall.py`、`p74_misc.py`、`p74_piglin.py`、`p74_fuzz.py` | 村莊與貓、交易大廳、末影人/海龜、豬布林規則、複製機 fuzz |
| `agent/W72Agent`+`W72Hook` | vanilla java agent;新增 `-Dw72.box=` 的盒內生成判斷計數、全域生成上限(MONSTER) |

## 踩到的坑(不是 Kiln 的問題)

1. `p72*.py` 裡寫死 `wp72` 目錄:`push_tools.sh` 用 sed 換成 `wp74`,否則 vanilla 那邊的指令寫進沒人讀的 fifo(`open` 永遠卡住)。
2. 重複指令方塊每刻 `kill` 會在 log 印 `Killed Zombie`,`kill_count` 的正規式把它當成自己的結果 → 假的「每回合 1 隻女巫」。要先 `gamerule command_block_output false` 並設 `TrackOutput:0b`。
3. **重複指令方塊與強制載入的區塊在 vanilla 會活過重啟,而且 `@e` 不帶座標時跨維度**:我留在世界裡的殺怪方塊把後來所有 vanilla 的村民、貓、傀儡殺光(村莊並排的 V 全是 0),Kiln 重啟後不再載入那些區塊所以沒事。每次跑完要拆方塊並 `forceload remove all`。**之後的下界/終界農場裡那個方塊的 `SuccessCount` 一直是 0(兩邊相同)**,所以黃金/末影人農場的 0 沒有意義(見「沒有結論」)。
4. 殺怪時別排除 `item`:掉落物累積後 Kiln 每刻 30 ms(4770 個物品),清掉之後每刻 2 ms。
5. 模擬距離 3 重啟後 vanilla 完全不生怪:怪物上限(`spawnableChunkCount` 固定 289 區塊)被重啟前留在「載入但不 tick」區塊裡的 100 隻怪佔滿(vanilla 的 `@e` 看不到它們)。之後都用模擬距離 6。
6. `par.sh` 不會轉送環境變數,參數要放在命令列;封閉池塘第一版把浮標卡在牆上,每次拋竿等滿 60 秒;`pkill -f 'fish_all'` 會殺掉自己的 ssh(要寫 `fish_al[l]`);`until ! pgrep -f …` 會比對到自己的指令列而永遠不結束。
7. 26.x 的實體 NBT 鍵是 snake_case(`has_egg`、`home_pos`),舊鍵名的選擇器永遠不符合(兩邊都一樣)。
8. 村莊/大廳腳本之前要 `respawn_all()`:前一個場景把玩家留在終界虛空,tp 指令對死亡的玩家沒有作用。
9. 測量傷害要先 `tick freeze`:取樣 85 隻生物要 30 多秒,兩個伺服器各自繼續跑,讀數不是同一時刻(「200 刻後」其實是 200~800 刻)。
10. 單元測試放方塊的位置要在方塊中心(玩家站在自己格子的中心,`fill` 用 `floor`,`summon` 用 `+offset`,差半格)。

## 玩了什麼、結果

### 1. 農場(同一個世界、種子 72;K = Kiln、V = vanilla)

| 農場 | 做法 | 結果 |
|---|---|---|
| 女巫小屋 | 小屋方塊盒 x -1281..-1271、z 735..745、y 58..72;每刻殺掉盒外所有實體(讓怪物上限空著),夜晚,模擬距離 6,**100 回合 × 1000 刻 = 10 萬刻**(vanilla 的 java agent 數到盒內 3000 刻只有 2 次女巫挑中、1 次有效位置) | K 0 / V 0。小屋範圍內的生成率低到 10 萬刻都沒有一隻;結構生成覆寫本身有 `structure_spawns` 向量(A) |
| 黃金農場(下界屋頂) | 15×15 石板平台 y 129..132;vanilla agent:盒內**有 94 個殭屍豬人有效位置、25 個豬布林、69 個終界使者**,代表 vanilla 的農場確實會生 | K 0 / V 0 — **無效**:農場的殺怪方塊在下界 `SuccessCount` 一直是 0(兩邊相同),怪物上限被盒外的怪佔滿;K 沒有 agent 可以數有效位置 |
| 終界使者農場(終界外島) | 同上,y 41..46 | K 0 / V 0,同樣無效(殺怪方塊另一個維度) |
| 末影人對末影蟎 | 36 組,末影人 + 末影蟎相距 5 格,每 50 刻數 | 第 50 刻剩 K 6 / V 4,第 100 刻全部 0:**相同**(`Enderman` 的 `NearestAttackableTargetGoal(Endermite, mustSee=true)` 與 Kiln 一致,javap 逐項對過) |
| 海龜(育種→產蛋) | 18 對,沙灘 + 5×5 池塘(有底),`InLove`,`home_pos`;每 100 刻數帶蛋的海龜 | 第 100 刻帶蛋的海龜 K 11 / V 12,3000 刻時仍帶蛋 K 4 / V 3,蛋塊 K 2 / V 1:**相同**(取樣少)。**但這個場景發現問題 1:海龜在 Kiln 會溺水** |

### 2. 取樣

* **釣魚**(兩個真客戶端同時,`tick rate 100`;`p74_fish.py`;浮標下沉偵測會漏掉約三成的咬餌,數的是拿到的物品):

| 變體 | K(魚/垃圾/寶藏) | V(魚/垃圾/寶藏) |
|---|---|---|
| 開放水域、Lure III + Luck of the Sea III、120 拋 | 56 / 2 / 5 | 57 / 3 / 3 |
| 開放水域、無附魔、70 拋 | 53 / 9 / **0** | 51 / 5 / 5 |
| 非開放水域(3×3 孔,兩側有牆)、無附魔、70 拋 | 55 / 6 / 0 | 60 / 4 / 0 |

非開放水域兩邊都沒有寶藏(`open_water` 條件),垃圾約 10%:相同。開放水域合併 K 寶藏 5/125、V 8/124(期望約 10,Luck III 的寶藏機率 ≈ 11%、無附魔 5%):**K 偏低但差異不顯著**(Fisher p≈0.4;K 自己對期望 p≈0.07)。沒有更多拋竿可跑;戰利品表本身由 `LootVectors` 驗證。
* **以物易物**(`p74_barter.py`,40 隻豬布林 × 50 輪,金錠掉在腳邊):K 1914 / V 1910 件,**χ² = 18.8,自由度 16**(p≈0.28):分布相同。
* **貓**(4 個尚未走過的村莊,12 × 2000 刻 = 一天):貓數 K 2→5(1 號村)、1、0、1;V 2→4、1、0、1;村民 6/6/12/3 兩邊完全相同,傀儡 1/1/1/1 相同,無職業村民 4/5/5/0(K)對 4/6/5/0(V)。相同。

### 3. 交易大廳、村莊

* 12 個 1×2 單間各放一座不同的工作站 + 一個無職業村民:兩邊都在 40 刻內 12/12 取得對應職業(認領)。
* 「偷」工作站:6 個村民、3 座工作站的一個房間,6 次試驗:兩邊每次都是 3 個有職業,每個 500 刻取樣。
* 補貨:給 12 個村民「已賣完」的交易(`uses:99`)並觀察兩天(4.8 萬刻):**兩邊都沒有補貨**(`RestocksToday` 從認領時的 1 不再變),代表這個單間設計沒觸發 `WorkAtPoi`;兩邊一致,但沒有得到補貨時序的比較。折扣(治療殭屍村民)沒做。
* 突襲(兩位玩家):**沒做**。`p66_raid.py` 是單一玩家,雙客戶端要兩個程序協調(兩個客戶端各連同一個伺服器,Kiln 做完再換 vanilla),時間超過預算;wp66 的單人完整突襲與英雄獎勵已涵蓋。

### 4. 下界

* **豬布林/豬獸規則**(真客戶端,`p74_piglin.py`,玩家 Resistance 4、關自然回血,每次拿 8 秒的血量軌跡):

| 情況 | K 受傷 | V 受傷 |
|---|---|---|
| 沒穿金裝,3 隻豬布林 | 8~10 | 11~13 |
| 戴金頭盔 | 0 | 0 |
| 戴金頭盔 + **真點開箱子** | **0 → 修好後 8.9** | 9.8 |
| 戴金頭盔 + 挖金磚/金礦(`p74_gold.py`) | 修好後 `angry_at` 出現 | `angry_at` 出現(ttl 差 1~2)|
| 豬獸(hoglin)旁邊 | 3.9 | 2.7 |
| 豬獸 + 扭曲菌 / 重生錨(排斥) | 0 / 0 | 0 / 0 |

箱子/金磚:**問題 3**。徒步走過要塞/堡壘遺跡:**失敗**(客戶端落地就被燒死/被打死,重生後在主世界走了一圈,沒有可用資料);只有生物普查和 MSPT 數字。
* 傳送門被生物打斷:沒有專門做。

### 5. 複製機

先試了查原版 26.3 是否仍可用:網路上的教學都是舊版或農場內容,沒有可用的 26.3 驗證,wiki 說 TNT 複製機 Java 1.14+ 有效、地毯/鐵軌複製機標「?」,只描述原理(TNT 方塊在活塞推動時被方塊更新「點燃」成實體而方塊本身還在;地毯複製機是黏液柱帶著珊瑚扇與地毯被活塞推拉),沒有座標與時序。所以改用**結構化 fuzz**(`p74_fuzz.py dupe2`):黏液柱 2~4 格高 + 黏性活塞 + 兩個觀察者時鐘/紅石塊/拉桿,周圍隨機放地毯、珊瑚扇、各種鐵軌、TNT、黏液、蜂蜜、觀察者、活塞、漏斗,隨機放一台礦車;兩邊每個檢查點(第 1、3、11、31 刻,後來加第 91 刻)比客戶端眼中的方塊、實體普查,最後比盒內物品**總數**。
* 230 個案例(80 + 150):**方塊與實體普查 0 差異**。
* 物品總數 K 比 V 多 1 件的情況約 1/3(`K 2 / V 1`…),但縮到最小(7 個指令)的案例重跑後兩邊相同(都是一件鐵軌),不是穩定可重現的差異(物品合併/計時雜訊,或 fuzz 腳本的 `kill` 與取樣順序);最大的一例 K 20 / V 15 縮小後 K 2 / V 1,同樣重跑相同。不下「複製機相同」的結論,只說在這個調色盤下沒找到穩定差異。
* 真正照著 wiki 描述蓋一台 TNT/鐵軌/地毯複製機(要精準的偏移與時序)沒有做。

### 6. 快樂恐懼幽靈四人騎乘

兩個真客戶端連同一個伺服器(Kiln 與 vanilla 各跑一次),`ride` 上去兩位玩家 + 兩個村民(座位 4),玩家 A 按 W 6 秒:
* 座位:K 玩家 (−0.65, 3.4, 1.57)、(−1.57, 3.4, −0.65),村民 (0.65, 4.0, −1.57)、(1.57, 4.0, 0.65);V 玩家 (0, 3.4, 1.7)、(−1.7, 3.4, 0)…。距離都是 1.7、高度 3.4/4.0,角度的差是幽靈隨機的初始偏航(`summon` 的偏航是隨機的)。
* 駕駛:ghast 每 0.5 秒前進 2.5 格,K 與 V 的軌跡逐點相同(100.5→128.6 對 103.1→131.7)。
* **機器人(kiln-bot)不能騎**(沒有實體互動封包),所以第 3、4 位用村民代替(`ride` 也可以把機器人玩家放上去,沒有做:vanilla 沒有機器人可比)。

## 找到並修好的問題

| # | 問題 | 修正 | 測試(拿掉修正會紅) |
|---|---|---|---|
| 1 | **海龜與銅傀儡在水下會溺水**(vanilla 的 `#can_breathe_under_water` 有 `minecraft:turtle`、`minecraft:copper_golem`)。全生物溺水稽核(87 種,每隻一個水槽,700 刻)兩邊只有這兩種不同 | `turtle.rs`、`copper_golem.rs` 的 `Info.breathes_under_water = true` | `kiln-sim/tests/mobs.rs::turtles_and_copper_golems_do_not_drown_but_cows_do`(拿掉:紅,已驗) |
| 2 | **生物不被點燃的營火、凋零玫瑰、粉雪傷害**:危害矩陣(87 種生物 × 10 種方塊,凍結後取樣)裡 vanilla 的生物被營火燒死、被凋零玫瑰害死、在粉雪裡每 40 刻凍傷(牛 −2、烈焰人/熔岩獸 −10,北極熊/流浪者/雪人免疫),Kiln 的生物完全沒事 | `inside.rs`:`Campfire`、`WitherRose` 的進入效果(`Action::CampfireHurt`/`WitherRose`,生物 tick 中途延後套用);`entity.rs`:`freeze_immune`(`#freeze_immune_entity_types` + `#freeze_immune_wearables`,另加 `Skeleton.canFreeze() == false` 與帶著吞下物品的硫磺方塊,最後這兩個是 `mob_parity` 的 `convert_skeleton` 向量逮到的:第一版讓普通骷髏也被凍傷,該向量的第 159 刻血量 19 對 20);`mob/mod.rs`:凍傷傷害(`tickCount % 40`、完全凍結、`#freeze_hurts_extra_types` ×5)、解凍條件 | `mobs.rs::mobs_are_hurt_by_campfires_wither_roses_and_powder_snow`(牛 8、烈焰人 10、北極熊 30、營火中的烈焰人 20…,與 vanilla 讀數逐項相同;拿掉:紅,已驗;骷髏例外由 `mob_parity` 的 `convert_skeleton` 守著) |
| 3 | **開箱子/終界箱/木桶/界伏盒、挖金磚/金礦不會激怒附近的豬布林**(vanilla:`PiglinAi.angerNearbyPiglins`;函式在 `piglin.rs` 早就有,只有船/礦車箱子呼叫它)。真客戶端:戴金頭盔點開箱子,vanilla 的豬布林攻擊(8 秒 9.8 傷害),Kiln 的毫無反應 | `blocks.rs`:`piglin_anger` 佇列;`container/open.rs`(開箱子)、`digging.rs`(`#guarded_by_piglins`)排進佇列;`entities.rs::anger_piglins`;`region.rs` 在方塊實體階段處理 | `mobs.rs::piglins_turn_on_a_player_who_opens_a_chest`、`piglins_turn_on_a_player_who_breaks_gold`(石頭不會,金磚會;拿掉:紅,兩個都驗過)。即時:真客戶端兩邊的 `angry_at` 記憶相同 |

稽核方法(`p74_drown.py`、`p74_hazard.py`)可重跑,是這次最有效的:一次比 87 種生物。危害矩陣剩下的差異是時序(±1 血、火的壽命隨機、女巫喝抗火藥水、潛影貝傳送)。

## 沒有結論或沒修的

| 項目 | 狀況 |
|---|---|
| 黃金/末影人農場 | 殺怪方塊在下界/終界不運作(兩邊 `SuccessCount` 0),計數無效。vanilla agent 證實盒內有大量有效生成位置;Kiln 側沒有等價的計數器 |
| 補貨/折扣/突襲兩位玩家/徒步下界/傳送門被打斷 | 見上,沒做完 |
| 釣魚寶藏機率 | K 在開放水域偏低(5/125 對 8/124),差異不顯著;要更多拋竿(或在 Rust 側直接取樣 `gameplay/fishing`) |
| 實體 NBT | Kiln 的 `attributes` 列出所有屬性,vanilla 只列有調整的(海龜:K 四筆、V 一筆);`Brain.memories` 在 Kiln 的 `data get` 對豬布林等常常是 `{}` |
| 比較器…等 | wp72 的「還沒修」三項仍在 |
| 生物上限是否把「載入但不 tick」區塊裡的怪算進去 | vanilla 算(見坑 5);Kiln 沒有測 |
| 冷知識 | `ride @e[tag=x] mount …` 在兩邊都因為多實體選擇器出錯,要加 `limit=1` |

## MSPT 與區塊

整個遊玩期間(Kiln 的 `mspt` 報告 176 筆,1 位玩家):平均的平均 1.23 ms、中位數 0.80 ms,只有一筆平均 > 5 ms(27 ms,p99 542 ms、max 1.6 s,時間點是危害矩陣的佈置階段,大範圍 `/fill` 加上上百個 `/summon`,推測但沒有逐項量;vanilla 沒有量)。區塊:最後一筆 gen_done=256、gen_latency 平均 359 ms(下界/終界長距離 tp,wp73 之後的管線正常)。**不尋常的事**:殺怪方塊的場景(每刻從 @e 殺掉數百隻、物品掉落不清)Kiln 每刻 30 ms 對 vanilla 8 ms,原因是 4770 個累積的物品實體(`EntityBox` 收集、`speculate`、`of_entity`);清掉物品後 2 ms。沒有修,但值得記:大量物品實體的每刻成本是 vanilla 的數倍。

## 截圖

`docs/wp74/hazards_KV.jpg`:兩個客戶端(大視窗 Kiln、小視窗 vanilla 的畫面疊在 Xvfb 上)看同一個場景:水槽裡的海龜與銅傀儡(修好後 800 刻仍活著)、點燃的營火旁被燒死的豬、粉雪格。

## 驗證(VM,最後一個 commit)

workspace 測試不帶與帶 `KILN_DATAPACK=/home/test/kiln/work/generated` 各 192 個二進位全部 `ok`;`determinism` 6/6;`mob_parity` 1462 個向量全過(第一輪 1461/1462:`convert_skeleton` 揪出普通骷髏不能凍傷,已修,見問題 2);`tools/parity_suites.py --skip-workspace` 102 個套件全部 exit 0。`crowd_golden` 沒有重錄(玩家封包、狀態 hash 都沒變)。`work/m6-mobs2/vectors.jsonl` 沒有動(備份 `.wp74bak`)。負向檢查(把修正退回上一版,對應測試必須失敗)四項全做:海龜/銅傀儡、營火/凋零玫瑰/粉雪、開箱子、挖金磚(`tools/wp74/neg2.sh`);骷髏例外的負向檢查就是第一輪的 `mob_parity` 失敗。
