# wp58:真實 Minecraft 26.3 客戶端完整生存通關紀錄

目的:用官方客戶端(不是 vanilla-vector harness)從頭玩一遍 Kiln,找出 harness 測不到的問題。
伺服器 `kiln` release(`KILN_GENERATOR=noise`、種子 58、`KILN_DATAPACK=~/kiln/work/generated`)與客戶端都跑在 VM 上
(使用者中途改要求:客戶端不在 Windows 上跑)。結果:**終界龍被打死**(進度「Free the End」)、蛋、出口傳送門、鞘翅滑翔都玩到了;
找到 4 個 Kiln 的問題,3 個修正並有測試,1 個修正但沒有測試(見下)。

## 環境與工具

| 項目 | 做法 |
| --- | --- |
| 伺服器 | `~/wp58_start.sh`:port 25585、世界 `~/wp58/world`、主控台是 fifo `~/wp58/console`、log `~/wp58/server.log` |
| 客戶端 | VM 上的 Xvfb `:99`(960×540)+ Mesa llvmpipe 軟體 OpenGL,Java 25,離線名 `KilnTest`;檔案由 `tools/wp58/vm_client_setup.py` 從 Mojang 官方 URL 抓(version json、Linux 規則的函式庫、資產),放在 `~/mc-client`;`tools/wp58/vm_client.sh` 啟動 |
| 輸入 | 26.3 客戶端用 SDL,視窗訊息打不進去,所以寫了 java agent(`tools/wp58/ClientCtl.java`):以 `-javaagent` 載入,在 127.0.0.1:25999 開行協定,從 render thread 呼叫遊戲自己的 `KeyboardHandler.keyPress`、`MouseHandler.onButton`、`charTyped`,並回報客戶端看到的狀態(位置、背包、螢幕、準心目標、周圍實體、容器欄位、方塊掃描) |
| 驅動 | `tools/wp58/ctl.py`(Python,跑在 VM):走路、挖方塊、真的點 GUI 欄位合成/熔爐/釀造/附魔/交易、放方塊(蹲著放,避免打開容器)、戰鬥、彈道計算射箭等 |
| 截圖 | `import -display :99 -window root`(只有 VM 自己的虛擬螢幕);精選的放在 `docs/wp58/*.jpg`,全部在 `work/wp58/` |
| 第二個玩家 | `kiln-bot`(`--behavior circle --chat-interval`),客戶端看到兩個玩家實體與聊天 |

為了省時間,下列情況用指令代替真實操作(都是「取得過程耗時、不測新東西」的):`/tp`(長距離移動,配 `slow_falling`)、
`/give`(黑曜石、打火石與鋼、盔甲、食物、眼珠、藥水瓶、羊毛…)、`keep_inventory`(我自己下錯指令死了很多次)、
`effect give resistance/fire_resistance`(打龍與下界時)。戰鬥、合成、挖礦、放方塊、開 GUI 都是真的客戶端操作。
客戶端因為軟體 OpenGL 很慢(準心目標每一幀才更新),腳本要等一下才瞄得準。

## 流程與結果

1. **新世界、砍樹、合成、挖礦(真實)**:噪音地形、樺木,客戶端渲染正常,無解碼錯誤。徒手砍原木 3.2 秒/塊(與 vanilla 相符),撿掉落物,背包 2×2 與工作台 3×3 合成(木板→工作台→木鎬→石器→熔爐→鐵鎬/劍→鑽石鎬/劍→桶),熔爐 GUI 燒鐵(進度、取出正常)。垂直礦井挖到 y=-56 找到 7 顆鑽石。`docs/wp58/furnace-gui.jpg`
2. **熔岩與水**:水桶放進熔岩洞,水流出隧道把玩家沖回去;沒有看到不合 vanilla 的流體行為。
3. **下界**:14 塊黑曜石一塊一塊放成門框(草會擋準心射線,先清草),打火石與鋼點燃,傳送門渲染正常,走進去 4 秒後到下界(座標 ÷8 正確),進度「Into Fire」「A Terrible Fortress」。堡壘裡兩個烈焰人刷怪籠都是 `minecraft:blaze`;打烈焰人得烈焰棒;豬靈交易(戴金頭盔、丟 3 個金錠,豬靈撿起停手、數秒後吐出皮革等)正常。`docs/wp58/portal-lit.jpg`、`blazes.jpg`、`piglin.jpg`
4. **村民、附魔、釀造、睡覺、農場、紅石、漏斗**:農夫村民交易 20 小麥換 1 綠寶石(MerchantScreen 按鈕);附魔台(4 個書架)附魔鑽石劍得到鋒利+擊退,消耗 2 經驗等級 2 青金石;釀造台把水瓶釀成粗劣藥水再釀成迅捷藥水(約 20 秒/步);夜晚睡床(重生點設定、進度「Sweet Dreams」、約 5 秒後天亮);鋤地、播種、骨粉長成熟小麥;黏性活塞被紅石方塊推動又收回;漏斗把箱子裡的圓石送進下面的箱子;船放在水上、上船、划 6 秒移動 40 格;礦車放在鐵軌上、上車(沒有推力所以不動,與 vanilla 相符);兩個 `kiln-bot` 玩家(聊天、加入/離開訊息、玩家實體)。
5. **夜間戰鬥**:蜘蛛、骷髏、豬靈、烈焰人、終界龍都殺過玩家,死亡畫面與訊息見問題 1、4。
6. **要塞與終界**:`/locate structure minecraft:stronghold` 在 (-1200, ~, 1056);丟眼珠飛向正確方向(位移比例與目標一致),掉落;要塞入口位置在 y≈-30 一帶(1.18+ 的 stronghold 很深,一開始我只掃 y≥0 而誤判成沒有生成)。傳送門房 12 個框架,把眼珠一個個放進去(失手會把眼珠丟出去)全部填滿,終界傳送門啟動;進度「The End?」。
7. **終界龍**:被弓箭打掉 2 個水晶(要算箭的下墜,直瞄會打到石柱側面),其餘水晶用指令移除;之後用附魔弓(力量 5、無限)與劍在約 9 分鐘內把龍從 200 打到 0,進度「Free the End」,出口傳送門(0,61,0)啟動,龍蛋在 (0,65,0)。`docs/wp58/dragon.jpg`、`end-island.jpg`
8. **出口傳送門、龍蛋、鞘翅**:出口傳送門回主世界(問題 3);龍蛋點擊會跳(問題 2);鞘翅在空中連按跳躍進入滑翔,煙火火箭加速(約 6 格/秒)。`docs/wp58/elytra.jpg`。終界城的船沒找到(`locate` 給的城堡 (-624, ~, -880) 一帶搜不到龍頭),鞘翅用指令給。

客戶端 log 全程沒有 WARN/ERROR/解碼錯誤(只有 Realms/認證/旁白這些與伺服器無關的行);伺服器 log 沒有警告。
伺服器 mspt:一個玩家加兩個 bot 時平均約 1.3~4 ms。

## 發現的問題與修正

### 問題 1:怪物殺死玩家的死亡訊息少了引數(已修正,有測試)

- 現象:被蜘蛛殺死,死亡畫面與聊天顯示 `%1$s was slain by %2$s`。截圖 `docs/wp58/death-unnamed-mob-windows-capture.jpg`(修正前);修正後 `docs/wp58/mobdeath.jpg`、「KilnTest was slain by Spider / Piglin」、「was shot by Skeleton」。
- 原因:`kiln-sim/src/entities.rs` 的 `hurt_player` 用 `current_info` 找攻擊者(正在 tick 的怪物,它的 `phys` 已被取出 list)。`current_info` 只有 `with_entity` 路徑會設定;一般實體 tick(`tick_entity`)、新乘客 tick、待處理 kill 都沒設,攻擊者變 `None`,訊息只剩一個引數的 `death.attack.mob`。
- 修正:三處設定 `sim.current_info`。
- 測試:`tests/mobs.rs::a_zombie_kill_names_the_zombie_in_the_death_message`(拿掉修正會失敗);`health.rs::mob_death_message_carries_its_arguments`(結構並用 vanilla 的 `ClientboundSystemChatPacket` codec 解碼確認)。

### 問題 2:龍蛋被點擊/打都不會跳(已修正,有測試)

- 現象:存活模式左鍵、右鍵點龍蛋,蛋不動。`kiln-blocks` 只有龍蛋的掉落延遲(5 tick),沒有 `DragonEggBlock.teleport`。
- 修正:`interact::use_without_item` 與新的 `interact::attack`(存活玩家開始挖方塊時呼叫,`digging.rs`)都會傳送:最多 1000 次嘗試,偏移 ±15 水平、±7 垂直,第一個空氣格。順便實作 `NoteBlock.attack`(打音符盒會響)。世界邊界不查(blocks 層沒有邊界資訊)。
- 測試:`kiln-blocks interact::tests::a_dragon_egg_jumps_when_clicked_or_punched`。
- 實機驗證:左鍵一下,蛋從 (0,65,0) 跳到 (7,62,1)。

### 問題 3:第一次從終界出口出來沒有片尾名單(已修正,有測試)

- 現象:走進出口傳送門,直接回到主世界,客戶端沒有顯示 credits。`ServerPlayer.showEndCredits` 的 `WIN_GAME` 事件值,看過是 0(客戶端直接要求重生)、沒看過是 1(客戶端先播 credits),Kiln 永遠送 0。
- 修正:`portal.rs::show_end_credits` 依 `seen_credits` 送 1.0 或 0.0。
- 測試:`tests/dimensions.rs::end_portal_platform_credits_and_return` 增加檢查第一次送的封包是 `game_event(4, 1.0)`。
- 沒有再用真客戶端驗證 credits 畫面(`seenCredits` 已經被我的第一次出口設成 true,而這個伺服器的玩家資料沒法在線改)。

### 問題 4:龍息雲殺死玩家的訊息有時沒有攻擊者名字(已修正,沒有測試)

- 現象:「KilnTest was killed by Ender Dragon using magic」正確,但同一場戰鬥後兩次死亡是 `%1$s was killed by %2$s using magic`。
- 推測的原因:`apply_instantaneous_effect` 只在**這個區域的實體清單**裡找雲的擁有者(龍);龍在別的區域時找不到,攻擊者變 None。
- 修正:`AreaEffectCloud` 記下 `owner_type`(龍息雲是 `minecraft:ender_dragon`),雲 tick 時透過新的 `EntityLevel::hint_owner_type` 告訴 sim,找不到擁有者時用這個型別與雲的位置當攻擊者。
- 沒有測試:要造出「雲在 A 區、龍在 B 區」需要很多佈置;修正只在原本會得到 None 的路徑上補值,不改其他行為。留待有空時用 region 分裂的 sim 測試。

## 不是問題但容易誤判的事

- 連點同一格 300 ms 內,客戶端視為雙擊(收集全部相同物品),會把合成格裡的物品吸走——vanilla 行為。
- 主控台 `/tp` 在主世界執行,要去下界/終界得用 `execute in <dim> run tp`;帶 `distance` 的選擇器要加 `x= y= z=`(以主控台位置為準)。
- 死在方塊裡/海裡時掉落物被推出方塊(下界推到 y=128 的基岩頂上);`/give` 在背包滿時掉在腳邊。
- 蹲著放東西(不然右鍵箱子/工作台會打開它);蹲下眼睛較低,瞄準要往下微調。
- 對沒有框架的地方右鍵眼珠會把它丟出去(浪費眼珠)。
- 傳送門房有熔岩池,框架在熔岩上方的平台上;房間裡的刷怪籠是蠹魚,幾秒內就能殺死沒盔甲的玩家。
- 終界出生平台 (100,49,0) 緊貼主島邊緣的終界石(平台把周圍挖成一個洞);世界生成的地形資料來自 vanilla datapack 並由 `kiln-worldgen/tests/dimensions.rs` 對照 vanilla,這裡沒有另外比對。
- 被指令傳到另一個維度的「已死亡」玩家(我自己的腳本造成),客戶端的死亡畫面會停住、重生按鈕一直灰色,重新連線才恢復;vanilla 的 `/tp` 對死亡玩家的行為我沒有驗證,不當成 Kiln 問題。

## 還沒做到 / 後續

- 終界城的船與鞘翅(`locate` 的城堡附近搜不到龍頭;可能這座城沒有船,要多找幾座)。
- 紅石農場、活塞門、熔爐陣列、釣魚、村莊襲擊;盔甲/武器的耐久與修補。
- 問題 4 的區域分裂測試;用真客戶端驗證 credits 畫面(需要一個沒看過 credits 的新玩家)。
