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
3. 殺光怪物時別排除 `item`:Kiln 與 vanilla 都會累積掉落物(Kiln 4770 個,每刻 30 ms),排除計數盒外所有實體後 Kiln 每刻 2 ms。
4. 模擬距離 3 重啟後 vanilla 完全不生怪:怪物上限(`spawnableChunkCount` 固定 289 區塊)被重啟前留在「載入但不 tick」區塊裡的 100 隻怪佔滿(vanilla 的 `@e` 看不到它們)。不是 Kiln 的問題;之後都用模擬距離 6。
5. `par.sh` 不會轉送環境變數,參數要放在命令列(`p74_fish.py N RATE POND ENCH`)。封閉池塘的第一版把浮標卡在牆上,每次拋竿等滿 60 秒。
6. `pkill -f 'fish_all'` 會殺掉自己的 ssh(樣式出現在自己的指令列),要寫 `fish_al[l]`。
