# wp58:真實 Minecraft 26.3 客戶端完整生存通關紀錄

目的:用官方客戶端(不是 vanilla-vector harness)從頭玩一遍 Kiln,找出 harness 測不到的問題。
伺服器 `kiln` release(`KILN_GENERATOR=noise`、種子 58、`KILN_DATAPACK=~/kiln/work/generated`)與客戶端都跑在 VM 上。

## 環境與工具

| 項目 | 做法 |
| --- | --- |
| 伺服器 | `~/wp58_start.sh`:port 25585、世界 `~/wp58/world`、主控台是 fifo `~/wp58/console`、log `~/wp58/server.log` |
| 客戶端 | VM 上的 Xvfb `:99`(960×540)+ Mesa llvmpipe 軟體 OpenGL,Java 25,離線名 `KilnTest`;檔案由 `tools/wp58/vm_client_setup.py` 從 Mojang 官方 URL 抓(version json、函式庫 Linux 規則、資產),放在 `~/mc-client` |
| 輸入 | 26.3 客戶端用 SDL,不吃視窗訊息,所以寫了 java agent(`tools/wp58/ClientCtl.java`):以 `-javaagent` 載入,在 127.0.0.1:25999 開一個行協定,從 render thread 呼叫遊戲自己的 `KeyboardHandler.keyPress`、`MouseHandler.onButton`、`charTyped`,並回報客戶端看到的狀態(位置、背包、螢幕、準心目標、周圍實體、容器欄位) |
| 驅動 | `tools/wp58/ctl.py`(Python,跑在 VM):走路、挖方塊、合成(真的點 GUI 欄位)、放方塊、戰鬥、熔爐等 |
| 截圖 | `import -display :99 -window root`(只有 VM 自己的虛擬螢幕) |

流程:`work/vm.sh script.py` 把腳本丟到 VM 執行。為了省時間,下列情況用指令代替真實操作並在各節註明:`/tp`(長距離移動,用 `slow_falling` 避免摔死)、`/give`(黑曜石、打火石與鋼、盔甲、食物——取得過程只是耗時,不測新東西)、`keep_inventory`(我自己下錯指令死了很多次)。

## 流程與結果

### 1. 新世界、砍樹、合成、挖礦(真實操作)

- 進入世界:噪音地形、樺木與草原,客戶端渲染正常,無解碼錯誤。
- 徒手砍原木(3.2 秒/塊,與 vanilla 相符)、撿掉落物、背包 2×2 合成木板/工作台、放置工作台、3×3 合成木鎬、石器、鐵鎬、鐵劍、鑽石鎬、鑽石劍、桶、熔爐。
- 挖石頭/煤/鐵/鑽石,垂直礦井挖到 y=-56,鑽石 7 顆;熔爐(GUI)燒鐵錠、進度與取出正常。
- 夜晚被蜘蛛打死(**問題 1**,見下)。
- 熔岩湖與水桶:水流出隧道,把我沖回去(行為合理);沒有死於 Kiln 的流體錯誤。

### 2. 下界(真實操作 + 指令移動)

- 黑曜石 14 個蓋門框(真的一個個放,草會擋準心射線,要先清草),打火石與鋼點燃,傳送門渲染正常,走進去 4 秒後到下界(座標 ×1/8 正確),取得進度「Into Fire」「A Terrible Fortress」。
- `/locate structure minecraft:fortress` 在 (160, ~, -144),進堡壘(進度正常),烈焰人刷怪籠 (162,70,-120)、(186,70,-133) 都是 `minecraft:blaze`。
- 打烈焰人 5 隻得烈焰棒(火焰抗性藥水期間火球不傷人,與 vanilla 相符)。
- 豬靈交易:戴金頭盔、丟 3 個金錠(Q 鍵),豬靈撿起、停手、數秒後吐出物品(皮革等),與 vanilla 相符。沒穿金裝的玩家 3 秒內被豬靈殺死,死亡訊息「was slain by Piglin」正確(修正之後)。

## 發現的問題與修正

### 問題 1:怪物殺死玩家時死亡訊息沒有引數(已修正,有測試)

- 現象:被蜘蛛殺死,死亡畫面與聊天顯示 `%1$s was slain by %2$s`(原始翻譯字串)。截圖:`docs/wp58/death-unnamed-mob.png`。
- 原因:`kiln-sim/src/entities.rs` 的 `hurt_player` 用 `current_info` 找攻擊者(正在 tick 的怪物,它的 `phys` 已被取出 list)。`current_info` 只有 `with_entity` 路徑會設定,一般實體 tick(`tick_entity`)、新乘客 tick、待處理 kill 都沒設,於是攻擊者是 `None`,訊息變成只有 1 個引數的 `death.attack.mob`,客戶端找不到第 2 個引數就顯示原始字串。(vanilla 的客戶端遇到引數不足也是這樣顯示。)
- 修正:三處設定 `sim.current_info`。
- 測試:`crates/kiln-sim/tests/mobs.rs::a_zombie_kill_names_the_zombie_in_the_death_message`(殭屍殺玩家,檢查封包含 `death.attack.mob` 與 `entity.minecraft.zombie`;拿掉修正會失敗);`health.rs::mob_death_message_carries_its_arguments`(訊息結構與 vanilla 的 `ClientboundSystemChatPacket` codec 解碼通過)。
- 驗證:修正後再被豬靈殺死,畫面顯示「KilnTest was slain by Piglin」。

## 不是問題但容易誤判的事

- 連點同一格 300 ms 內,客戶端視為雙擊(收集全部相同物品),會把合成格裡的物品吸走——是 vanilla 行為,腳本要間隔 >300 ms。
- 主控台 `/tp` 在主世界執行,要去下界得用 `execute in minecraft:the_nether run tp`;帶 `distance` 的選擇器要加 `x= y= z=`。
- 死在方塊裡/海裡時掉落物會被推出方塊(下界推到 y=128 的基岩頂上)。

## 待完成

(依序:村民交易、附魔、釀造、睡覺、農場、紅石、漏斗/熔爐陣列、船與礦車、多人、要塞、終界傳送門、終界龍、龍蛋、出口傳送門、終界城鞘翅。)
