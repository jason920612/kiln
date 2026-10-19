# wp84:第十一次遊玩(快樂恐魔、騎乘與長矛、不祥試煉、考古與陶罐、凋零與傳送門、物品×生物矩陣、資料包伺服器;玩家的伺服器端速度)

分支 `wp84-playthrough11`(從 main `82165ceb` 開出)。方法沿用 wp72~wp83:**比較的兩邊必須是同一個世界**(種子 84、noise 產生器、視距與模擬距離 8、同一組資料包)、
樣本不夠不下結論、鎖步(`tick freeze`+`tick step`)能得到位元精確的就不用統計、vanilla 自己不決定性的(跨區塊特徵)不當依據。
所有建置與測試只在 VM(`~/kt wp84-playthrough11 …`,`CARGO_BUILD_JOBS=4`、`RUST_TEST_THREADS=6`);Kiln 25589、vanilla 25595;另一個 session(wp85)用 25599/25596,沒有碰它的程序與檔案。
工具在 `tools/wp84/`(`push_tools.sh` 複製到 VM 的 `~/wp84`)。

這一輪最有用的新東西:**客戶端自己的速度封包進了 spy**(`ClientCtl` 新增 `motion_self`:收到的、屬於本機玩家的 `SetEntityMotion` 與其數值)。
沒有它,擊退、長矛突進這些「客戶端自己動」的差異只能從落點間接推;有了之後,殭屍打一下、長矛戳一下兩邊送的向量直接並排,找到了這一輪最大的問題(見「問題 5」)。

## 工具(`tools/wp84/`)

| 項目 | 做法 |
|---|---|
| `p84_cmp.py` | 兩邊跑同一支 par.sh 腳本之後:`facts/<場景>_K|V.txt` 逐行比、`spy/` 的封包集合比(`-q` 只印不同的) |
| `p84_ghast.py`、`p84_gfollow.py` | 快樂恐魔:幼體跟隨(雪球/木棍/空手)、餵食(每次真實點擊後的 Age)、成體吃雪球、掛具(真點擊、染色、剪刀、潛行)、繩子(圍籬、牽著走) |
| `p84_spear.py`、`p84_lunge.py` | 長矛:騎馬時戳/衝刺、突進 I~III(地面、連戳、空中);`p84_lunge.py` 逐 50 ms 的位移與速度封包 |
| `p84_trial.py`、`p84_ois.py` | 不祥試煉:喝不祥之瓶 → 不祥之兆 → 試煉之兆 → 刷怪籠不祥化、掉落物生成器、保險庫與鑰匙;指令召喚的不祥物品生成器的封包 |
| `p84_mace.py` | 重錘從 H 格落下打 NoAI 殭屍(密度、突破、風爆)——**這個驅動打不到空中的一擊,沒有結論**(見下) |
| `p84_arch.py` | 兩邊 `locate` 結構、兩個客戶端飛過去,`findbox` 可疑沙/礫/陶罐/箱子的位置與方塊實體資料(鎖步) |
| `p84_pots.py` | 裝飾陶罐:放入、同物品加一、不同物品搖頭、空手、比較器、漏斗進出、鎬/絲綢/箭打碎、合成 |
| `p84_jockeys.py` | 裸 `/summon <型別>`(跑 `finalizeSpawn`)N 次:騎乘者、幼體、裝備的統計(鎖步,兩邊各一個客戶端) |
| `p84_wither2.py` | **兩個真玩家**同一台伺服器打一隻凋零(`switch.sh K|V both`):血條、傷害、骷髏、半血階段 |
| `p84_portal.py`、`p84_kb*.py`、`p84_eat.py`、`q_*.py` | 傳送門(殭屍在門裡打玩家、豬)、擊退封包(含凍結逐刻)、吃東西、各種一次性查詢 |
| `p84_matrix.py`+`p84_matrix_cmp.py`、`p84_gallery.py` | 物品×生物矩陣、封包畫廊(從 wp80 複製) |
| `p84_dp.py`、`packs/make_kw84.py`、`packs/kw84/` | 資料包 `kw84`(自己寫的小 JSON):函式(`#minecraft:load`/`tick`、巨集、巢狀 `return`)、述詞、7 個進度、loot 與 recipe 覆寫、4 個附魔、傷害類型、tag |
| `neg84.sh`/`neg84.py`、`verify84.sh` | 負向檢查(把修正用文字編輯關掉,測試必須失敗)、最後驗證 |

## 踩到的坑(不是 Kiln 的問題)

1. **兩個客戶端 + 兩個伺服器開著編譯,OOM killer 殺掉 `rustc`**(`signal 9`,測試「編譯失敗」):編譯與測試前一律 `stop_clients.sh`。
2. 物品×生物矩陣每組約 8~10 秒(軟體算圖 + 兩個伺服器 + 另一個 session 的編譯):第一次想跑 1280 組停掉,縮成 17 種生物 × 12 種物品 = 204 組(約 25 分鐘)。
3. `/summon <型別> … {任何 NBT}` **不跑 `finalizeSpawn`**(只有裸 id 才跑),所以統計騎乘者必須裸召喚;vanilla 遠離玩家的怪**立刻消失**(超過 128 格),Kiln 不會——統計場地要放在玩家身邊。
4. Windows 上沒有 `python3`(只有商店的假指令):改檔用 `…\Python311\python -I`,heredoc 裡的 `\n` 跑兩次轉義會把字串弄斷(`neg84.py` 兩次壞掉才發現)。
5. `ssh … pkill -f 樣式` 會殺掉自己的 ssh(樣式也在指令列裡)——用腳本檔、PID。
6. 26.3 的資料包格式:loot/進度/述詞的條件鍵是 `type`(不是 `condition`)、實體種類在述詞裡是 `minecraft:entity_type`、進度的 `player`/`entity` 是單一條件(不是陣列)、`time_check` 需要 `clock`、`repair_with_xp` 的元素要 `{effect:…}`、參照述詞是**字串**(沒有 `minecraft:reference` 型別);vanilla 只要有一個元素壞掉就**整個伺服器拒絕啟動**(`can't proceed with server load`)。
7. `advancement test` 不是指令(用 `execute if entity @s[advancements={…=true}]`);`experience set … points` 之後 `XpTotal` 不一定歸零(比增量,不比總量)。
8. 打凋零時兩個玩家的目標選擇是亂數:一次執行的受傷曲線不能比,只比血條與總量。

## 玩了什麼、結果(K = Kiln、V = vanilla)

### 1. 快樂恐魔(`p84_ghast.py`、`p84_gfollow.py`,真客戶端)
* **幼體跟隨**:拿雪球站著,幼體從 9 格外飛來,到約 2.1 格停下;木棍、空手也會靠近到 2~3 格(`BabyFollowAdult` 的玩家目標):修好後兩邊的距離曲線逐段相同(K 5.4/3.0/2.1、V 6.1/3.7/2.2)。修好前 K 停在 6.6 格(**問題 2**)。
* **餵食**:幼體每餵一個雪球 Age 約 −24000 → −21512 → −19324 → −17358 → −15591(兩邊同);成體:V 不收雪球也沒有愛心,K 吃掉並送 `entity_event 18`(**問題 1**)。
* **掛具**:真點擊裝上(紅色)、再點另一色=上去坐(不是替換)、潛行點=不上去也不換、剪刀拆下並還給物品、幼體不能穿:封包集合相同,**缺上/下坐騎的護目鏡音效**(**問題 3**)。
* **繩子**:圍欄上/牽著走:拉力範圍 K 太緊(**問題 4**);修好後距離曲線同量級。
* **站在背上不動、四人同騎**:wp57/wp68/wp74 做過;這一輪用 `tp` 把玩家放在背上,兩邊的恐魔都在 1 秒內飄走(玩家落地),沒有可比的結果。**兩個客戶端同騎**用 wp68 的腳本在 wp68 做過,沒有重跑。

### 2. 騎乘與長矛(`p84_spear.py`、`p84_lunge.py`、`p84_jockeys.py`、矩陣)
* **突進**:地面 I/II/III 位移 K 1.21/2.43/3.64、V 1.00/2.01/3.02,連戳兩次 K 又多走 0.25、V 沒有;空中 K 3.3/6.6/9.9、V 2.3/4.6/6.9。逐 50 ms 位移 + 新的速度封包:兩邊送的向量只差 **y**:V `(0, −0.0784, 0.458)`、K `(0, 0, 0.458)`——**問題 5**。
* **騎馬長矛**(馬速固定、NoAI 豬):戳與衝刺兩邊都能打;一次取樣的豬血量不同(命中時機),沒有結論。
* **鸚鵡螺、殭屍鸚鵡螺、駱駝屍殼、殭屍馬、骷髏馬**:矩陣(下)+ 畫廊;**騎乘者統計**(`p84_jockeys.py`,各 200~600 次裸召喚):駱駝屍殼、殭屍、屍殼、溺屍、蜘蛛、骷髏、乾屍、沼澤骷髏、雞、殭屍馬、豬獸、豬布林、殭屍豬布林的幼體率、武器率、護甲率、騎乘者(蜘蛛騎士 V 1/169、雞騎士 K 0/V 1)全部在抽樣雜訊內(殭屍武器率 K 5/600、V 4/600;屍殼 7 對 5)。**天氣雷暴的骷髏陷阱馬、野外的駱駝屍殼+乾屍騎手**沒有自然生成可觀察(需要等亂數),`skeleton_horse.rs`/`camel.rs` 與 javap 對過(wp68);沒有新的差異。
* **溺屍騎殭屍鸚鵡螺**:`finalizeSpawn` 沒有騎乘者(兩邊 N=200 都是 0),沒有進一步的場景。
* **畫廊**(`p84_gallery.py`,breeze、parched、bogged、camel_husk、zombie_horse、skeleton_horse、happy_ghast、zombie_nautilus、creaking、sulfur_cube、copper_golem 各 8 秒):幾乎全同;差異:殭屍鸚鵡螺**在地上的環境音/受傷音 K 的檔名不存在**(**問題 10**)、微風 K 多一個 `ProjectilePower`(vanilla 只對火球類送;沒修,無客戶端效果)、駱駝屍殼 K 多一個 `camel_husk.sit` 音(取樣一次,沒查)。

### 3. 不祥試煉與重錘(`p84_trial.py`、`p84_mace.py`)
* **完整流程**(真客戶端,`/place structure trial_chambers` 同一處):喝不祥之瓶(`ominous_bottle_amplifier=1`)→ Bad Omen 放大 2 → 站進刷怪籠房間,**20 刻內 Bad Omen 變 Trial Omen**、刷怪籠 `active, ominous=true`,不祥物品生成器掉箭/風彈/藥水、蜘蛛與殭屍出現,保險庫與鑰匙(一般與不祥)開箱:兩邊相同。差異:**喝完不祥之瓶 vanilla 播 `item.ominous_bottle.dispose` 給喝的人,K 沒有**(**問題 6**);物品生成器的實體資料 vanilla 在配對後又送一次(K 一次;客戶端沒有可見差別,沒修)。
* **重錘**:從 6/12/24 格往 NoAI 殭屍掉,同時按住攻擊並每 40 ms 瞄準——驅動打不到下落中的一擊(兩邊都常常落地才打),**沒有結論**;重錘的 `hurtEnemy`/`getAttackDamageBonus`/擊退半徑與 Kiln 的程式逐行對過 javap(沒有差異)。

### 4. 考古與陶罐(`p84_arch.py`、`p84_pots.py`)
* `locate`:沙漠神殿 `(−768,160)`、足跡遺跡 `(~−318,~−269)`、冷海遺跡 `(688,128)`、暖海遺跡 `(−544,160)` 兩邊相同;可疑沙/礫、箱子、陶罐、TNT 的位置**相同**(沙漠神殿 8 塊可疑沙 + 9 TNT、足跡遺跡 96 塊可疑礫、冷海遺跡 6 塊可疑礫 + 28 個陶罐(其實是同處的試煉密室)),`LootTable` 名稱相同。**不同**(兩次都重現):沙漠神殿正下方 `(−769,−35,160)` K 多一個怪物房(地牢)刷怪籠;暖海遺跡 K 多 6 塊可疑沙(V 同位置是沙或水)。屬於世界產生(wp85 的範圍),沒有拿 vanilla 對 vanilla 比,**記錄,沒修**。沙漠水井:Kiln 不產生(wp83),V 這個種子附近沒有可看的。
* 陶罐(真點擊):放入(同物品 +1、物品與數量逐次相同)、不同物品與空手(搖頭,`block_event 1 1` 與 `insert_fail`)、漏斗從下面抽(25 → 9,6 秒)與從側面灌(9 → 16)、鎬打碎(掉內容物)、絲綢之觸(掉陶罐)、箭打碎(`level_event 2001`)、合成(破片/磚排成菱形 → `pot_decorations` 逐面相同):**全部相同**;差異:**比較器**(**問題 7**)。

### 5. 凋零、監守者、傳送門
* **兩個真玩家打一隻凋零**(`p84_wither2.py`,同一台伺服器,75 秒、半血階段):兩邊兩個客戶端的血條**逐項相同**(`Wither`、紫色、`PROGRESS`、變暗、進度 0.57667),凋零回血曲線 143→174 同斜率,玩家傷害總量同量級;目標分配是亂數(K 的 A 玩家 28 秒後不再受傷、V 持續受傷),一次取樣沒有結論。
* **傳送門**(`p84_portal.py`,殭屍在門裡打玩家、豬站在門裡):換維度時間 5.5/4.0/4.0 秒(K)對 5.4/4.0/4.1(V)、殭屍在門裡打:4.0 對 3.95~4.0、豬 10.5 秒後離開:**全部相同**。被擊退的方向與強度:見問題 5(修好後在門裡被打也一樣)。
* **監守者潛行**、**挖出古城**:wp78 做過潛行與姿勢直方圖;這一輪**沒有重做**(時間用在問題 5)。

### 6. 物品×生物矩陣(`p84_matrix.py`,真客戶端 NoAI 生物 2 格外、右鍵,17 種生物 × 12 種物品 = 204 組)
快樂恐魔、鸚鵡螺、殭屍鸚鵡螺、駱駝屍殼、殭屍馬、骷髏馬、騾、蜜蜂、鸚鵡、悅靈、村民、鐵/雪/銅傀儡、哞菇、行者獸、豬獸 × (小麥、胡蘿蔔、骨頭、金蘋果、鱈魚、桶、剪刀、命名牌、鞍、繩、刷子、蜜脾):**168 組相同、31 組不同**:
* 悅靈(11 組):給物品時 K 送 `allay.item_given`、V 不送(`level.playSound(player, this, …)` 排除玩家本人)→ **問題 8**。
* 駱駝屍殼/殭屍馬/殭屍鸚鵡螺 + 繩(3 組):V 拴上、K 不收 → **問題 8**。骷髏馬 + 鞍:V 裝上、K 不收 → **問題 8**。
* 哞菇 + 剪刀:V 有 `explosion` 粒子 → **問題 8**(掉落蘑菇數 K 2/V 5 是取樣太早,沒查)。
* 雪傀儡(5 組點擊落空)、行者獸(12 組只差 `Health` 的浮點呈現)、`snow_golem` + 剪刀(K 的 `data get` 讀到錯誤文字,是腳本的雜訊):不是 Kiln 的問題。

### 7. 資料包伺服器(`p84_dp.py`、`kw84`;兩邊同一包,生存、真客戶端)
* **載入**:`#minecraft:load` 的函式跑一次(`#loads` 1 對 1)、`#minecraft:tick` 每刻跑(分數 K/V 相同曲線:拿金錠的玩家每刻 +1,粒子 70 個/3 秒);巨集 `function kw84:macro {who,item,amount}`(給 3 個綠寶石、`tellraw`、`#macro`=3)、巢狀函式 `return`(`#r`=5、`#inner`=1)、述詞(拿金錠真/木棍假、`time_check` 14000 真):**全部相同**。
* **進度**:3 個金粒 → `nuggets` 的 toast/聊天/XP 37/loot(綠寶石、紫水晶碎片)/配方書 4 個/獎勵函式;吃麵包(隱藏挑戰)、`either`(木棍或骨頭,`requirements` 的 OR)、`pig_kill`(帶述詞 `player`):**相同,除了問題 9**(K 拒載 `pig_kill`)。
* **loot 覆寫**:豬(綠寶石 ×2 + 豬肉 3~5,殺的方式要玩家)兩邊相同(豬肉數是亂數);泥土 `blocks/dirt` 的覆寫**兩邊都沒有生效**(掉的還是泥土;我的 alternatives 寫法在 26.3 的 block loot 不被採用,兩邊一樣,沒有再查)。
* **配方**:棍子覆寫(×7)、竹塊走 `#planks` 的 tag(×14)、無序合成(泥土×2+金粒 → 鑽石 ×2,由進度解鎖)、燻烤熔爐(礫石 → 玻璃 ×3,12 秒 12 個):相同。
* **附魔**:`venom`(對殭屍 +2 傷害 +中毒,真揮劍:97→89.096 / 98→85.652 兩邊逐位相同)、`plating`(護甲保護 +3 −、屬性 `movement_speed` 加成,10 點傷害扣 6.0,兩邊同)、`miner`、`looter`(載入):相同。
* **tag**:`mineable/axe` 加泥土(鐵斧挖泥土 K 0.76 s/V 1.03 s,鏟 0.51/0.46 s:真人按鍵時間雜訊)、`climbable` 加玻璃(兩邊沒爬——客戶端要貼著牆跳才會爬,腳本只往前走,**沒有結論**)。
* **傷害類型** `kw84:laser`:兩邊的 `foodExhaustionLevel` 都沒變(同);死亡訊息沒測。

### 找到並修好的問題

| # | 問題 | 修正 | 測試(拿掉修正會紅,`neg84.sh`) |
|---|---|---|---|
| 1 | **成體快樂恐魔吃雪球**:`animal_interact` 沒問 `canFallInLove`,雪球被收走、送愛心事件 | 成體回 `PASS`(`HappyGhast.mobInteract` 之後的 `Animal.mobInteract`) | `happy_ghast_wp84.rs::a_grown_happy_ghast_does_not_take_the_snowball` |
| 2 | **由 NBT 載入的恐魔幼體(`/summon … {Age:-24000}`、存檔重啟後的幼體)被當成體**:`AgeableMob.setAge` 越過 0 才會跑的 `babyGhastSetup`(拿掉目標、換成腦)沒跑,幼體用成體的 `ForNonPathfinders` 在 7 格外停住、不會跟人 | `HappyGhast::load` 在幼體時跑 `baby_setup` | `a_ghastling_from_saved_data_follows_a_snowball_like_a_baby` |
| 3 | **缺 `harness_goggles_down/up` 音效**,放下最後一個乘客時沒有 `serverStillTimeout = 10`、沒清除家 | `post_tick` 追蹤乘客數:第一個上去 → 護目鏡下音、沒有玩家站在背上就解除靜止;任何人下去 → 靜止 10 刻,最後一個 → 護目鏡上音 + `clearHome` | `a_harnessed_happy_ghast_plays_its_goggles…` |
| 4 | **牽著的快樂恐魔拉得太緊**:繩子彈性距離 6/斷裂 12(`Leashable` 預設),快樂恐魔/恐魔是 10/16;`shouldStayCloseToLeashHolder` 是 false(近距離不走向持繩者);`onElasticLeashPull` 要 `setWait()`;家的半徑 `elastic−1` | `leash.rs`:`snap_distance`/`elastic_distance_of`、不走近、拉扯時 `Wait` | `a_leashed_happy_ghast_keeps_its_lead_out_to_16_blocks_not_12` |
| 5 | **玩家的伺服器端速度**:vanilla 的 `ServerPlayer` 每刻跑 `travel`(站著 Motion y = −0.0784;擊退的封包送的是**走一步之後**的值:殭屍打人 y 0.275、水平 0.218;剩的在幾刻內消失)。K 有兩個欄位(`vel` 給擊退/衝擊、`server_delta` 給 phantom 身體),擊退從 0 起算、送原始的 0.4/0.4、剩的不衰減(連打會累加,實測 −0.40 → −0.53);長矛突進從 y=0 起算 → 客戶端以為不在地上,地面摩擦少一階,**走 1.21 格而不是 1.00** | 兩個欄位併成 `vel`(`phantom_travel` 每刻走它);**擊退封包等一刻再送**(我們的玩家階段在實體階段之前,下一刻 travel 走完才送,等於 vanilla 的「實體階段 → 玩家 travel → sendChanges」);非 travel 的狀態(旁觀、飛行、滑翔、騎乘、睡覺、死亡)照舊衰減;玩家存檔的 `Motion` 寫真值;`state_hash` 只含水平分量(垂直是依周圍方塊的重力,拓撲間會不同);等區塊的玩家不再二次衰減 | `player_knockback_wp84.rs`(殭屍打:水平 0.2184、y 0.2752,40 刻後再打一次同值;站著 Motion y −0.0784);真客戶端:殭屍打站著的玩家 K/V 的 `motion_self` 都是 `(−0.0090, 0.27516, −0.2182)`;`crowd_golden` 四個**沒有重錄**(狀態雜湊與封包摘要都沒變) |
| 6 | **喝不祥之瓶沒有 `item.ominous_bottle.dispose`**(`PlaySoundConsumeEffect` 是 `playSound(null, …)`,連喝的人都聽到;K 只送給別人) | `consume.rs`:`queue_sound_for_all_at`(方塊位置的中心、`PLAYERS` 音源) | `ominous_bottle_sound.rs` |
| 7 | **裝飾陶罐放第二個以後比較器不更新**(只有 0 → 1 那次有) | `decorated_pot.rs` 放入後 `update_neighbour_for_output_signal`(vanilla `setTheItem` → `setChanged`) | `pot_comparator.rs`(44/64 讀 10) |
| 8 | **矩陣的四個**:駱駝屍殼/殭屍馬/殭屍鸚鵡螺不能拴(`canBeLeashed` 是「不是 Enemy」);骷髏馬不收鞍(`SkeletonHorse.canUseSlot` 對所有欄位都 true,先於 `mobInteract`);悅靈給/取物音是**排除給的人**的實體音;哞菇剪完沒有 `explosion` 粒子 | `leash.rs`、`horse.rs`、新事件 `EntitySoundExcept`(`allay.rs`)、`mooshroom.rs` | `interact_wp84.rs`(4 個) |
| 9 | **進度的 `player` 條件寫成述詞檔案的名字(字串)被拒絕載入**(`expected an object, found a string`),整個進度不見 | `criteria.rs::cap` 接受字串(`LootItemCondition.CODEC` 是 `either(holder, direct)`) | `advancement_reference.rs` |
| 10 | **殭屍鸚鵡螺/鸚鵡螺離水的音效 id 是 `…ambient_on_land`,真正的 id 是 `…ambient_land`**(常數叫 `*_ON_LAND`):環境、受傷、死亡、衝刺、衝刺就緒的離水音都不存在 → 客戶端收到不存在的音效(K 的 `generic.hurt`、沒有環境音) | `nautilus.rs::on_land` | `nautilus.rs::land_sounds::the_land_sounds_are_registered`(拿 registry 對) |

## 沒有結論或沒修的

| 項目 | 狀況 |
|---|---|
| 重錘從空中打下的傷害/密度/突破/風爆 | 驅動打不到;kiln-sim 的 `spear_parity`/`melee_parity` 與 wp68 的一次取樣(12 格 11 點,兩邊同)是證據 |
| 兩玩家凋零的目標分配、`SetPassengers` 封包數(vanilla 上/下坐騎各 2 個,K 各 1 個)、微風 `ProjectilePower`、駱駝屍殼 `sit` 音 | 客戶端沒有可見差別或一次取樣,沒修 |
| 指令 `/damage` 的擊退封包:vanilla 在指令(實體階段之外)產生的是**未走步**的原始值,K 一律等一刻、走一步 | 遊玩裡的傷害(怪、箭)才是常見來源且已對;指令的來源沒做 |
| 快樂恐魔當牽繩持有者(`supportQuadLeashAsHolder`:四點彈簧拉船、馬、羊駝、嗅探獸)、`leashHolderTime` | **沒做**(`checkElasticInteractions` 的四點版要整套 `Wrench` 移植) |
| 世界產生:沙漠神殿正下方多一個地牢、暖海遺跡多 6 塊可疑沙 | 記錄(wp85 在做世界產生) |
| `data merge block` 改陶罐/箱子的物品不通知比較器(V 會) | 沒修(指令佈置才會遇到) |
| 資料包 `blocks/dirt` 的 loot 覆寫兩邊都沒生效、`climbable` 玻璃沒爬 | 腳本/寫法問題,兩邊一致,沒再查 |
| 監守者潛行、被打斷的傳送門(玩家被打斷後回到原維度)、挖出古城 | 沒重做(wp78 做過;這一輪的時間用在問題 5) |

## MSPT 與區塊

最後一次啟動(截圖場景,一位玩家)的 Kiln `mspt` 報告 5 筆:平均約 1.3 ms、單刻最大 181 ms(載入場景的 `fill`/`summon`);沒有 > 400 ms 的單刻。另外一次玩中觀察:`tp` 到沒生成過的遠處(暖海遺跡)時 `chunk_wait` 平均 7.4 s、最大 10.8 s(`gen_terrain_ms` 累計 27 萬 ms,VM 負載 10 以上、另一個 session 在編譯),vanilla 同樣要等(wp73、wp80 記過的冷生成停頓)。
玩家速度每刻多走一次 `travel` 的成本:`phantom_travel` 本來就每刻跑,只是不再被後面的 `decay_velocity` 重複處理(對非 travel 的玩家才呼叫);`sim_load` 沒有重測(沒有新的每刻工作)。

## 截圖

`docs/wp84/ghast_pot_KV.jpg`:傍晚,掛具的快樂恐魔、一隻幼體與一個裝了 20 個紫水晶碎片的裝飾陶罐(Kiln 大視窗、vanilla 小視窗疊在上面);右邊的計分板是資料包 `kw84` 的 `#minecraft:tick` 函式在加的分數(兩邊都是 106 左右)。

## 驗證(VM,最後一個程式碼 commit `wp84: nautilus sounds out of water are *_land`;之後只改文件與腳本)

見 `docs/parity-coverage.md` §6.27 的最後一段(數字在那裡)。
