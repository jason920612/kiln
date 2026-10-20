# wp86-mgmt:管理伺服器(vanilla 26.3 的 `net.minecraft.server.jsonrpc`)

分支 `wp86-mgmt`(從 `wp86-server-ops` 開出並併回它的新提交,只推到 `vm`)。做法承襲 wp86:**用 `javap` 讀 vanilla 26.3 的位元碼**(類別:`jsonrpc.*`、`notifications.*`、`players.*List`、`DedicatedServer`、`MinecraftServer`、`ServerLoginPacketListenerImpl`),**真的跑一台 vanilla 26.3**(VM 的 `~/wp86m/van`,管理埠 25666)和一台 Kiln(`~/wp86m/kil`,管理埠 25669),拿同一批請求打兩邊、比原始文字。沒有看過、更沒有抄 Pumpkin、SteelMC、Paper、Folia、Bukkit 的任何程式。Mojang 的位元碼與 `rpc.discover` 的傾印只留在 VM 的筆記裡,沒有進 repo;repo 裡的資料表(`mgmt/spec.rs`)是從協定的行為寫出來的方法名、說明字串與結構。

## Management server

### 怎麼用

`server.properties`(鍵、預設值與驗證和 vanilla 的 `DedicatedServerProperties` 一樣,由 wp86 的 `properties.rs` 讀寫):

| 鍵 | 預設 | 意思 |
|---|---|---|
| `management-server-enabled` | `false` | 開啟 |
| `management-server-host` | `localhost` | 綁定的位址(空字串 = 所有介面;主機名稱會解析,IPv4 優先) |
| `management-server-port` | `0` | 埠;`0` = 系統選一個空的(啟動時的記錄會寫出真正的埠);超出 0~65535 啟動失敗 |
| `management-server-secret` | 缺少時產生 40 個英數字寫回檔案 | 驗證用;必須剛好 40 個英數字(`SecurityConfig.isValid`),否則啟動失敗 |
| `management-server-tls-enabled` | `true` | TLS(`wss://`);關掉就是明文 `ws://` |
| `management-server-tls-keystore` / `-password` | 空 | PKCS12 檔(第一個私鑰項目與它的憑證鏈);密碼先看環境變數 `MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD`,再看屬性 |
| `management-server-allowed-origins` | 空 | 以逗號分隔;只用在把密鑰放在子協定的連線(瀏覽器) |
| `status-heartbeat-interval` | `0` | 每幾秒送一次 `server/status` 通知;0 = 不送 |

建一個測試用的 keystore:

    keytool -genkeypair -alias mgmt -keyalg RSA -keysize 2048 -validity 365 -dname CN=localhost \
        -storetype PKCS12 -storepass changeit -keypass changeit -keystore mgmt.p12

連線:`GET /`(只有路徑 `/` 是 WebSocket),帶 `Authorization: Bearer <密鑰>`;瀏覽器沒辦法設標頭,所以也可以用 `Sec-WebSocket-Protocol: minecraft-v1, <密鑰>` 加上一個在 allowed-origins 裡的 `Origin`(回應會帶 `sec-websocket-protocol: minecraft-v1`)。每則訊息是一個文字框,內容是 JSON-RPC 2.0 的物件或批次(陣列)。例子(`tools/wp86/mgmt/wsrpc.py` 是我手寫的小用戶端):

    $ python3 tools/wp86/mgmt/wsrpc.py 127.0.0.1 25669 <密鑰> \
        '{"jsonrpc":"2.0","id":1,"method":"minecraft:allowlist/add","params":[[{"name":"Alex"}]]}'
    <- {"jsonrpc":"2.0","id":1,"result":[{"id":"…","name":"alex"}]}
    <- {"jsonrpc":"2.0","method":"minecraft:notification/allowlist/added","params":[{"id":"…","name":"alex"}]}

`rpc.discover` 回傳完整的 OpenRPC 文件(68 個方法、21 個通知與元件結構,欄位與順序和 vanilla 一樣)。方法名可省略 `minecraft:`(`allowlist` = `minecraft:allowlist`)。參數可以放在陣列(剛好一個元素)或物件(用參數名)。

**方法**(都在 `mgmt/spec.rs`;`/set` 把整張表換成給的,`/add` 加,`/remove` 去掉,`/clear` 清空,都回傳新的整張表):

| 群組 | 方法 |
|---|---|
| 允許名單 | `allowlist`、`allowlist/{set,add,remove,clear}`(玩家 `{"id","name"}` 兩個都可省略,有 id 用 id 查,否則用名字) |
| 封鎖 | `bans`、`bans/{set,add,remove,clear}`(`{"player","reason","source","expires"}`,`source` 預設 `Management server`,`expires` 是 ISO-8601 時刻) |
| IP 封鎖 | `ip_bans`、`ip_bans/{set,add,remove,clear}`(`add` 可以給 `ip` 或一個在線上的 `player`;不合法的位址被略過) |
| 玩家 | `players`、`players/kick`(`{"player","message"}`;`message` 是 `{"literal"}` 或 `{"translatable","translatableParams"}`) |
| 管理員 | `operators`、`operators/{set,add,remove,clear}`(`{"player","permissionLevel","bypassesPlayerLimit"}`,等級夾在 0~4) |
| 伺服器 | `server/status`、`server/save`(`[flush]`)、`server/stop`、`server/system_message`(`{"message","overlay","receivingPlayers"}`) |
| 設定 | `serversettings/<名>` 與 `…/set`:`autosave`、`difficulty`、`enforce_allowlist`、`use_allowlist`、`max_players`、`pause_when_empty_seconds`、`player_idle_timeout`、`allow_flight`、`motd`、`spawn_protection_radius`、`force_game_mode`、`game_mode`、`view_distance`、`simulation_distance`、`accept_transfers`、`status_heartbeat_interval`、`operator_user_permission_level`、`hide_online_players`、`status_replies`、`entity_broadcast_range`(`/set` 改完會寫回 `server.properties`,和 vanilla 一樣) |
| 遊戲規則 | `gamerules`、`gamerules/update`(`{"key","value"}`;布林或整數,整數有範圍) |

**通知**(沒有 `id`,所有連線都收到;同一條連線上,**回應先、它引起的通知後**):`server/started`(初始存檔的 `saving`、`saved` 之後)、`server/stopping`、`server/saving`、`server/saved`、`server/activity`(線上模式有人通過驗證;最多每 30 秒一次)、`players/joined`、`players/left`、`operators/added|removed`、`allowlist/added|removed`、`bans/added|removed`、`ip_bans/added|removed`、`gamerules/updated`、`server/status`(心跳,伺服器還沒啟動完時 `started` 是 `false`)、`world/upgrade_*`(Kiln 沒有世界升級,從不送)。通知出在名單與規則本身,所以**指令也會觸發**:`/whitelist add`、`/op`、`/ban`、`/ban-ip`、`/pardon`、`/gamerule`、`/save-all`、`/stop` 和玩家進出,管理連線都收得到。

**啟動與停止**:`Main.main` 一開始(世界載入前)就起管理伺服器,所以世界還在載入時只有 `rpc.discover` 能用,其他方法回 `-32600 Method cannot be dispatched pre server initialization: <方法>`;伺服器就緒後才有 `server/started`。`/stop`、`server/stop` 或 Ctrl-C 時,通知是 `stopping`、`saving`、`saved`,然後連線關閉。記錄行和 vanilla 一樣:`Starting json RPC server on <host>:<port>`、`Json-RPC Management connection listening on <host>:<實際埠>`、每條連線 `RPC Connection #<n>: Management connection opened for /<位址>`、`RPC Connection #<n>: <動作>`(例如 `Add player 'x' to allowlist`、`Update max players from 20 to 5`、`Game rule 'keep_inventory' updated from 'false' to 'true'`)。啟動失敗(密鑰不對、埠超出範圍、位址解析不了、埠被占、TLS 設不起來)記 `Failed to start the minecraft server` 與 vanilla 的例外訊息(TLS 失敗另外記五行說明),行程以 1 結束。

### 實作

| 檔案 | 內容 |
|---|---|
| `crates/kiln-server/src/mgmt/` | `mod.rs`(生命週期、`Ctx`、啟動檢查)、`conn.rs`(HTTP 升級、驗證、連線迴圈,通知寫在回應之後)、`ws.rs`(HTTP/WebSocket 框,netty 的握手檢查與錯誤文字)、`rpc.rs`(JSON-RPC 與錯誤碼、分派)、`exec.rs`(所有方法)、`codec.rs`(DFU codec 的錯誤文字、UUID/Instant 解析)、`json.rs`(Gson 風格的寬鬆解析與 `toString`)、`spec.rs`(方法表與 `rpc.discover`)、`hub.rs`(通知、心跳計時器)、`jhash.rs`(Java 的雜湊與 `HashSet` 迭代順序)、`tls.rs`(PKCS12 → rustls) |
| `crates/kiln-link/src/mgmt.rs` | `Notification`、`Notifier`、`MgmtHost`(模擬端的臉:在線玩家、踢人、op、設定、遊戲規則、存檔、停止、系統訊息);`access.rs` 的名單改成有 `whitelist_add`、`ban_add`、`op_add`… 這些會通知的方法(指令和管理方法都走它們) |
| `crates/kiln-sim/src/mgmt.rs` | `MgmtHost for Sim`;`lib.rs`:`started`/`stopping`/`saving`/`saved`/`players/left|joined` 的通知,等下一個 tick 的時候就處理管理與 RCON 的工作(下面) |
| `crates/kiln-server/src/main.rs` | 在載入世界前啟動、`onServerExit` 時停止 |
| `tools/wp86/mgmt/` | `wsrpc.py`(用戶端)、`probe.py`(拿一份腳本打伺服器、印原始文字)、`run.sh`、`compare.sh`、`auth_probe.py`、`pre_init.py`、`bench.py`、`scripts/01~07` |

方法在連線的執行緒上解碼參數、查玩家;碰到遊戲的(踢人、op、設定、規則、存檔、停止)送成 `ToSim::Admin` 的工作在模擬執行緒上做(vanilla 也是在主執行緒做)。**模擬的迴圈原本只在 tick 開頭處理 Admin 工作**,一個要問遊戲的請求會等上一整個 tick(50 ms);現在等下一個 tick 的空檔就隨到隨做(只在 `lockstep` 排程,這時沒有區域在跑),和 vanilla 的主執行緒在等 tick 時做排隊工作一樣。RCON 也因此快了。

## 與 vanilla 並排比對

方法:同一份腳本(`tools/wp86/mgmt/scripts/`)打 vanilla 和 Kiln,印出每一則**原始文字**(回應、通知、第二條觀察連線收到的通知),`compare.sh` 把會因時間或 vanilla 本身不穩的行拿掉後 `diff`。vanilla 用 `-Dminecraft.api.*.host=http://127.0.0.1:9` 讓名字查詢快速失敗,測試名字都用 `kt-…`(Mojang 沒有的)。

| 腳本 | 內容 | 請求數 | 結果 |
|---|---|---|---|
| `01_basic` | 協定錯誤(壞 JSON、各種 `id`/`method`/`params`、批次、`rpc.discover`)、所有 getter | 65 | 一致 |
| `02_lists` | 允許名單、封鎖、IP 封鎖、管理員的增刪改,指令引起的通知 | 49 | 一致 |
| `03_server` | `server/*`、所有設定的 `/set`(邊界值、型別錯誤、`1e20`、`3000000000`、`-1`…)、遊戲規則、`/stop` | 97 | 一致(收尾處兩行是腳本關連線的時間差) |
| `04_edge` | 方法名格式、UUID 各種壞法、Instant 各種壞法、`permissionLevel`、遊戲規則 key、訊息、`id` 的重複鍵/HTML 字元 | 81 | 一致 |
| `05_players` | 真的連進一個用戶端:進出通知、系統訊息(封包逐位元組)、踢人、封鎖/IP 封鎖/強制允許名單踢線上玩家、規則 | 30 | 一致(除了進出的聊天訊息,見下) |
| `06_parse` | 44 種不是 JSON 或 Gson 寬鬆讀得下的文字 | 44 | 完全一致(含 `line/column/path` 與 `Troubleshooting` 連結) |
| `07_tls` | 同樣的請求走 TLS(keytool 的 PKCS12)與心跳 | 3 | 一致 |
| `auth_probe.py` | 57 種握手:Bearer 的各種寫法、子協定與 Origin、非 GET、路徑、版本 7/8/13/其他、缺鍵、`Expect: 100-continue`、第一版草案、垃圾、LF 結尾… | 57 | 逐位元組一致 |
| `pre_init.py` | 伺服器還在啟動時連進去:前置錯誤、`rpc.discover`、心跳(`started:false`)、`saving → saved → started → started:true` | 3 | 一致 |

幾個從位元組碼讀出來、再用上面證實的細節:

- 錯誤碼與文字:`-32700` 的 `data` 是 Gson 例外的整段文字;`-32602` 的 `data` 是 DFU codec 的訊息(多個欄位出錯時**後面的欄位在前**,`Not a string: 6; Not a string: 5`);`id` 照原文回(`1.50` 就是 `1.50`),只接受 null、數字、字串;`method` 不是字串但是基本型別就用它的文字找;連 `[1,2]` 這種批次元素不是物件會直接關連線。
- Java 數字:整數參數 `3000000000` → `-1294967296`、`2.5` → `2`、`1e20` → `1661992960`(`Number.intValue`);權限等級與廣播範圍夾在範圍內再寫回屬性。
- `UUID.fromString` 的八種失敗訊息(`Error at index 15 in: "…"`、`UUID string too large`…)、`Instant` 的解析(`Text '…' could not be parsed at index 10`、有毫秒的 `…00.123Z`)。
- `set` 方法比較的是集合,**移除與新增的順序是 Java `HashSet` 的迭代順序**(record 的 `hashCode`、`UUID`/`String`/`Instant` 的雜湊、桶的擴張):`bans/set` 先移除 grief2、grief3、grief5 這個順序是 vanilla 實測出來的(`jhash.rs` 重現)。
- 名單的順序是 vanilla 的 `HashMap` 順序(wp86 的 `JavaHashOrder`)。
- 同一個玩家 `allowlist/add` 兩次會通知兩次;`remove` 不存在的不通知;`clear` 對每個項目通知一次移除。
- `players/kick` 回傳之前玩家就已經離開名單(`players/left` 先於斷線);`bans/add`、IP 封鎖、強制允許名單的踢人在 vanilla 是下一個 tick 離開,Kiln 立刻離開(外面看不出差別,通知照樣在回應之後)。

測試(都在 VM 跑,`--release`):`kiln-server` 的 `mgmt::` 34 個(JSON/codec 單元測試、真的 socket 上的協定/驗證/握手/通知/設定/TLS〔keytool 做的 PKCS12〕/心跳/批次/框,用一個假的模擬),`kiln-sim` 的 `tests/mgmt_host.rs` 7 個(真的 `Sim`:進出通知、踢人、op、規則、設定、強制名單、存檔與系統訊息)、`kiln-link` 的 `changes_are_announced`;`cargo test --release -p kiln-server -p kiln-link -p kiln-sim --lib`(60 + 6 + 128)、`-p kiln-command -p kiln-net --lib`(119 + 62)、`kiln-sim --test server_ops --test mgmt_host`(14 + 7)全過,`cargo check --workspace --all-targets` 沒有警告。每個新行為的測試在拿掉該行為時會失敗(例如拿掉 `HashSet` 順序,`set_removes_in_the_order_of_a_java_hash_set` 就失敗)。

**回應時間**(`bench.py`,一條連線循序 2000 個請求,同一台 VM):

| 請求 | vanilla | Kiln |
|---|---|---|
| `serversettings/motd`(讀屬性) | 6 255 req/s,p50 134 µs | 24 425 req/s,p50 41 µs |
| `players` | 10 382 req/s,p50 76 µs | 30 034 req/s,p50 32 µs |
| `serversettings/autosave`(問遊戲執行緒) | 10 109 req/s,p50 80 µs | 22 028 req/s,p50 44 µs |
| `serversettings/motd/set`(寫 `server.properties`) | 3 002 req/s,p50 265 µs | 13 259 req/s,p50 66 µs |

(改模擬迴圈之前,問遊戲執行緒的請求是 20 req/s,p50 50 ms,正好一個 tick。)

## 和 vanilla 不同的地方(與原因)

| 項目 | vanilla | Kiln | 原因 |
|---|---|---|---|
| 遊戲規則的順序 | `gamerules` 列出的順序每次啟動都不一樣(以規則物件的身分雜湊排) | 規則表的順序 | 無法重現;內容、型別、值一樣 |
| 用名字找玩家 | 先找線上、名單、快取,最後向 Mojang 查(離線模式也查,查不到才用離線 UUID) | 線上、名單、已查過的;線上模式向工作階段服務查;離線模式不連網,直接給小寫名字的離線 UUID(和 wp86 的指令一致) | Kiln 離線模式不連 Mojang;真帳號的名字在離線模式得到離線 UUID |
| `view_distance`/`simulation_distance` | 立刻套用到在線玩家 | 存進設定與屬性檔,對之後的加入與載入生效;已在線的客戶端不重送 | 模擬沒有執行期改視距的路徑 |
| 進出的聊天訊息 | 可翻譯的 `multiplayer.player.joined`(帶點擊與懸停事件) | 黃色的純文字 `X joined the game` | wp86 之前就有的差異,不在管理伺服器裡(`05_players` 的比對排除這幾行) |
| `server/save` 的 `flush` | 控制是否強制寫盤 | 存檔流程固定(同步寫入由儲存層決定) | Kiln 的存檔管線自己決定;通知與回傳值一樣 |
| 第一版草案(沒有版本標頭、有 Origin 的 Hixie)握手 | 回 101 並改用草案的框 | 回同樣的 101,連線保持但不處理草案的框 | 沒有人用 |
| `world/upgrade_*` | 世界升級時送 | 從不送 | Kiln 沒有 DataFixer 式的世界升級 |
| TLS 版本與套件 | JDK 的 | rustls:TLS 1.2/1.3 | 平台差異 |
| 執行緒 | netty 事件迴圈(`Management server IO #n`) | 每條連線一個執行緒,名字相同 | 簡單;管理連線很少 |

## 還沒做

- 草案(Hixie)WebSocket 的框;世界升級通知;離線模式下向 Mojang 查名字。
- `view_distance` 的執行期重送(需要模擬支援)。
- TLS:多個私鑰項目時只用第一個(Java 的 KeyManager 依別名與金鑰型別選)。
