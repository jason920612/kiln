# wp65:第五次真實客戶端遊玩(兩個真實客戶端同時在線 + vanilla 26.3 參考伺服器並排)

分支 `wp65-playthrough5`(從 main `c174a46a` 開出)。做法同 wp58~wp64:官方 26.3 客戶端跑在 VM 的 Xvfb + llvmpipe 上,由 `tools/wp58/ClientCtl.java`(java agent)驅動,
**這次同時有兩個真實客戶端**(演員 `KilnTest`:agent 25999;第二位玩家 `Obs`:agent 26000,都是真的玩家,不只是旁觀者)。
伺服器:Kiln(`KILN_GENERATOR=noise`、種子 65、`KILN_DATAPACK=~/kiln/work/generated`,port 25587)與 vanilla 26.3 參考伺服器(同種子、port 25593),
**同一份種子兩邊的地形、結構位置、箱子戰利品種子完全一致**(見下),所以可以直接並排比對。指令只用在佈置(平台、`give`、`tp`、`tick rate`、`kill` 清場),
實際的戰鬥、右鍵、開 GUI、騎乘、按鍵、走進傳送門都走客戶端。

## 工具(相對 wp64 的變動)

| 項目 | 做法 |
| --- | --- |
| 驅動 | `ClientCtl` 新增 `ent EID`:客戶端眼中另一個實體的位置、姿勢、四件護甲、主/副手、使用/格擋、揮手動畫、載具與乘客 |
| 兩個客戶端 | `tools/wp58/p65.py`(`clients()`、`ent()`、`go()`、`mspt()`、`chunks()`);`vm_observer65.sh` 可調畫質(`OBS_FPS/OBS_RD`),第二位玩家也是完整玩家 |
| 場景腳本 | `p65_b1.py`(PvP、聊天、丟物品、船、睡覺)、`p65_b2.py`(下界傳送門連結)、`p65_n1/n2.py`(下界要塞)、`p65_spawn.py`(自然生怪統計,`TARGET=K|V`)、`p65_end.py`(終界閘門)、`p65_elytra.py`(鞘翅+煙火火箭,量 mspt 與區塊)、`p65_glide*.py`、`p65_strider.py`、`p65_chest.py`、`p65_horse_e.py` |
| 啟動 | `wp65_start.sh`、`wp65_vanilla.sh`、`vm_client65.sh`、`vm_observer65.sh`、`build_agent_wp65.sh` |

踩到的坑(不是 Kiln 的問題):`pkill -f` 的樣式出現在自己的 ssh 指令列上會殺掉自己的 shell(用 `[p]attern` 寫法);主控台 `/tp` 在主世界執行,在下界要 `execute in minecraft:the_nether run tp`;
站在傳送門裡不離開會讓玩家傳送冷卻一直補滿(vanilla 也是);`import p64` 會把 `VDIR` 指到 wp64;resistance V 的玩家把 `effect clear` 後摔死,之後所有 `tp` 都是對死人;
vanilla 伺服器中途自己消失過一次(沒有日誌),用 `van2.sh` 在原世界續跑。

(進行中,見下方各節。)
