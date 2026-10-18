# wp80:第十次遊玩(海底神殿、終界、林地府邸與前哨站、考古、快樂恐魔、騎乘、蒼白之園、信標與銅、試煉)

分支 `wp80-playthrough10`(從 main `302681f4` 開出)。方法沿用 wp72~wp79:**比較的兩邊必須是同一個世界**(種子 80、noise 產生器、視距與模擬距離 8)、
樣本不夠不下結論、鎖步(`tick freeze`+`tick step`)能得到位元精確的就不用統計、vanilla 自己不決定性的(跨區塊特徵)不當依據。
所有建置與測試只在 VM(`~/kt wp80-playthrough10 …`,`CARGO_BUILD_JOBS=3~4`);Kiln 25589、vanilla 25595。工具在 `tools/wp80/`(`push_tools.sh` 複製到 VM 的 `~/wp80`)。

## 工具(`tools/wp80/`)

| 項目 | 做法 |
|---|---|
| `lock.py`、`lab80.py` | wp78 的鎖步工具 + 實驗室輔助:`tank`/`box`、`nbt()`(兩邊 `data get` 去掉前綴)、`count_blocks()`(用 `fill … replace` 的數量計方塊)、`cmpn()` |
| `par.sh`、`switch.sh`、`actor.sh`、`wp80_*.sh`、`agent/` | 從 wp78 複製、改名 wp80(Kiln 預設種子 80、vanilla `VSEED` 80) |
| `p80_guard.py`、`p80_elder.py` | 守衛者雷射(水槽內的生存玩家,每 5 刻取樣生命)、長老守衛者的挖掘疲勞(效果的出現時間、持續時間、重新施加,客戶端的 `game_event 10`) |
| `p80_monument.py`、`p80_conduit.py` | 海綿(吸水、植物、半磚、地獄烘乾)與導管(框架 15/16/29/41/42、能量、範圍、對敵對生物的傷害) |
| `p80_hurt.py`、`p80_proj.py`、`p80_gallery.py`、`p80_one.py`、`p80_lines.py`、`p80_fall.py` | **封包畫廊**:44 種怪物在封閉場地(障礙方塊牆)對生存玩家 12 秒,比較兩邊客戶端收到的封包種類/音效/粒子(只有 K 有、只有 V 有的),再用 `p80_lines.py` 看單一封包 |
| `p80_struct.py` | 兩邊 `locate` 結構是否同座標,載入後的生物普查 |
| `p80_allay.py` | 悅靈跳舞(唱片機)與紫水晶複製(真點擊) |
| `p80_pale.py` | 眼眨花:1600 朵,日夜與黃昏/黎明邊界 |
| `p80_bonemeal.py` | 骨粉(用發射器),每種植物 16~40 個格子的方塊數統計 |
| `p80_gateway.py`、`p80_crops.py`、`p80_matrix.py`、`p80_beacon.py`、`p80_small8.py` | 閘道、作物生長、物品×生物互動矩陣、信標、羅盤/激流 |

## 踩到的坑(不是 Kiln 的問題)

1. **`~/wp80/agent` 被 `build_agent_wp80.sh` 的 `rm -rf` 清掉**(同一個目錄放 vanilla agent 原始碼與客戶端 agent 的輸出):改輸出到 `clientagent/`。
2. **重啟 Kiln 時預設種子是 72**(`wp80_kiln_keep.sh` 的 `${KILN_SEED:-72}` 從 wp78 複製過來):結構位置兩邊不同(`locate` 全部 DIFF),查到才砍世界重來。
3. **建置時 vanilla 伺服器被 OOM killer 殺掉**(VM 只有 10 GB、另一個 session 也在跑):之後的「V」數字讀的是死伺服器的舊 log(全 0)。建置前先停 vanilla 與客戶端。
4. `fill` 超過 32768 格靜默失敗(導管水池、作物排)。
5. **`instant_health` 振幅 30**:`4 << 30` 在 Java 是 0,什麼都不補;用 10。
6. vanilla 的光照引擎在 `fill` 後不是立刻更新:同一刻放的作物(匹克植物)會被「光線不足」拆掉(Kiln 的光照同步更新)——測試場地要先填、等一下、再放作物。
7. `heredoc` 裡有 `'` 的長 Python 在這個 shell 會被截斷;用 Write 工具寫檔。
8. 沒有玩家時 Kiln 的主控台指令與發射器 tick 很慢(整個腳本 7 分鐘沒有輸出):要連著客戶端。

(待補:玩了什麼、結果、問題與修正、沒做的、MSPT、截圖、驗證。)
