# Kiln 插件 API（`kiln:api` 1.0）

本文件是 wp51 的成果：先質疑「插件到底需要什麼」，刪掉不需要的，再蓋出最簡單、能涵蓋剩下用途的 API。
執行緒契約（設計文件 §11.2–§11.4）不變：沒有主執行緒、沒有同步的跨 region 呼叫、跨情境只用型別化原子操作、
擁有權命名空間、失敗關閉（fail-closed）。實作現況與量測見 `docs/design-v2-regionized.md` §11.7；
WIT 在 `wit/kiln-api.wit`（遊戲熱路徑）與 `wit/async-tasks.wit`（非同步工作）。

## 1. 方法

1. **質疑需求**：列出真實伺服器靠插件做的事，逐項寫出它真正需要哪些事件與呼叫。
2. **刪**：沒有任何用途需要的、或能用更小的東西組出來的，就不做（§3，附理由）。
3. **簡化**：剩下的只做一種形狀——事件進來、動作（effect）出去，動作不在處理器裡生效。
4. 才開始蓋（§4 起），每個用途至少有一個會跑的範例插件，範例都在測試裡跑（§7）。

## 2. 用途與需求

| 用途 | 真正需要的事件 | 真正需要的呼叫 | 決定 |
| --- | --- | --- | --- |
| 保護／領地 | 可取消：`block-break`、`block-place`（對方塊使用物品，含開箱、開門、倒水）、`entity-interact`、`entity-attack`、`player-damage` | cell 範圍資料（領地）、`deny-message`、`chat.send` | 保留。`entity-attack`、`player-damage` 是新增（動物保護、PvP 區） |
| 經濟 | `join`／`leave`、`op-results`、指令 | global 原子操作（`add`、`compare-and-set`、`append`，新增 `try-add`：餘額不足就不套用）、玩家範圍資料、`chat.send`／`chat.tell` | 保留；`try-add` 是新增，因為「扣款但不得變負」用 CAS 迴圈在多 region 下會一直重試 |
| 聊天／格式 | 可取消 `chat`（取消或改寫） | 玩家名稱、`global-get`（唯讀快照，例如前綴表） | 保留，不變 |
| 權限 | 可取消 `command`、host 端 `bypass-permission` 過濾 | 無 | **部分刪除**：只保留「用指令事件否決」；不做權限節點提供者、不讓插件改 op 等級（見 §3） |
| 小遊戲／競技場 | `player-died`、`player-spawned`（觀察）、`player-damage`、`custom`（插件互相詢問）、工作排程 | `teleport`、`set-game-mode`、`give`、`take`、`clear`、`heal`、`kill`、`set-blocks`（競技場還原）、HUD | 保留 |
| 自訂物品／GUI／選單 | 可取消 `item-use`（右鍵手上的物品）、可取消 `container-click`（選單與一般容器） | `give`（名稱、說明、插件標籤、模型、閃光）、`open-menu`／`set-slot`／`close-menu` | 保留；選單一律鎖定（點擊不會搬動物品），插件只看到點擊 |
| 計分板／HUD | 無（由玩家事件或工作驅動） | `title`、`action-bar`、`sidebar`、`bossbar`（逐玩家，不動伺服器計分板） | 保留；逐玩家虛擬側欄，不碰共享計分板 |
| NPC | `entity-interact` | `entities.spawn`／`remove`（只能動自己生成的）、實體範圍資料 | 保留 |
| 反作弊 | `player-died`／`player-damage`（觀察速率）、玩家狀態查詢 | `info`（位置、血量、模式、是否在地面…）、`for-player` 週期工作取樣、`kick` | **部分刪除**：不提供每次移動／封包 hook（見 §3）；改成「週期取樣玩家狀態」 |
| 世界編輯 | 位置工作（`at-position`） | `set-blocks`（批次，限於呼叫所帶的 cell） | 保留寫入；**刪除** `get-block`（見 §3）；選區、結構檔讀寫在 guest 內自己做 |
| 傳送／家 | 指令、`player-damage`（取消暖機） | `info`（目前位置）、`teleport`、`for-player` 工作、玩家範圍資料、`online` 目錄 | 保留 |
| 記錄／稽核 | `block-break`／`block-place` 觀察批次、`join`／`leave` | `log`、`fs.data` | 保留，不變 |
| 跨插件 API（Vault 之類） | `custom` | `events.raise`（同一情境內、同步、可取消） | 保留，用單一個 `custom` 事件做完 |
| 外部世界（Webhook、Discord、資料庫） | 無 | HTTP、計時器、簡單儲存 | 保留，在獨立的 `async-tasks` world（§6），只用訊息接觸遊戲 |

## 3. 刪掉的東西與理由

- **每次方塊更新、每次實體 tick 的 hook**：沒有任何上面的用途需要；成本是每 tick 幾萬次跨邊界。
- **每次移動、封包層級 hook（`packet.observe`）**：反作弊的真實需求是「取樣玩家狀態」，`info` 加週期工作就夠，而且不會讓
  guest 看到未驗證的封包。需要時以後加（只增，不破壞）。
- **權限節點系統與改 op 等級**：Kiln 只有 0–4 等級；插件要自己的權限節點，就用 `command` 事件否決加自己的玩家／global 資料。
  讓插件改 op 等級等於讓任何插件取得全伺服器權限。
- **同步的跨 region 呼叫、跨 region 讀取、共享 KV 的 get/put**：契約禁止（v1 已刪）。
- **自訂世界生成、自訂配方、自訂附魔**：用 datapack，插件不需要。
- **直接改共享計分板**：逐玩家虛擬側欄能做 HUD；共享計分板需要的人用 `/scoreboard`。
- **`get-block`（讀事件之外的方塊）**：保護類插件需要的資料（領地）本來就在自己的 cell 資料裡；要把方塊讀進 guest 得把 region 的
  cell 指標借給呼叫（要 `unsafe`），或每次事件預取一個方塊盒（每次幾 µs，全部插件付費）。兩者都不值得；世界編輯類在 guest 內
  以觀察到的變更自己記帳。以後若需要，可用「位置工作回傳方塊快照」只增不破壞地加入。
- **從處理器同步取得其他玩家／實體的狀態**：只有事件帶的玩家、cell、實體可讀；其他人經 `online` 目錄取得名稱與 uuid，再用動作或訊息。
- **「立即生效」的動作**：所有動作都排隊，在序列點（P 階段之後、指令之後、B0 的工作之後）依確定順序套用。處理器裡的 `teleport` 不會讓同一個
  處理器看到玩家已移動——這是故意的：處理器可以在 trap 時乾淨地放棄，動作不留下部分結果，也不需要跨 region 加鎖。

## 4. 最終 API 表面（WIT `kiln:api@1.0.0`）

### 4.1 事件（guest 匯出）

region 實例（每個 region 一個，綁 region 不綁執行緒）：

| 匯出 | 種類 | 在 sim 的哪裡 |
| --- | --- | --- |
| `on-block-break` / `on-block-place` | 可取消 | 玩家封包（挖方塊、對方塊使用物品、水桶類）進原版處理之前；`block-place` 涵蓋開箱、開門 |
| `on-entity-interact` | 可取消 | 右鍵實體；可讀寫該實體的插件資料 |
| `on-entity-attack` | 可取消 | **新**：玩家打非玩家實體（`Attack` 封包，進戰鬥處理之前）；可讀寫實體資料 |
| `on-player-damage` | 可取消 | **新**：`Player::hurt` 在確定「這一下會打中」之後、動任何狀態之前（所有傷害來源；`/kill`、虛空等繞過無敵的傷害不問） |
| `on-item-use` | 可取消 | **新**：`UseItem`／`UseItemOn`，在 `block-place` 之前；host 只在物品帶該插件的標籤、或訂閱 `items` 過濾符合時才呼叫 |
| `on-container-click` | 可取消 | **新**：`ContainerClick`；插件選單一律鎖定（回傳值被忽略、點擊被吃掉、畫面重送），一般容器需 `vanilla = true` |
| `on-chat` / `on-command` | 可取消 | 取消或改寫 |
| `on-custom` | 可取消 | **新**：其他插件用 `events.raise` 發出的事件 |
| `on-observe` | 批次 | 方塊破壞／放置；**新**：`player-died`（死亡訊息宣告的序列點）、`player-spawned`（加入、重生、換世界，玩家有 region 之後的第一個 B0） |
| `on-task` / `on-results(player, results)` | B0 | 工作與原子操作／動作／工作（job）的結果；**新**：結果帶著來源玩家 |

global 實例（每個插件一個）：`init`、`on-enable`、`on-disable`、`on-join`、`on-leave`、`on-command`（註冊的指令）、`on-task`、
`on-results`、`on-cancelled`、**新**`on-custom`。

### 4.2 呼叫（guest 匯入，依 manifest capability 連結）

| 介面（capability） | 呼叫 |
| --- | --- |
| `state`（永遠） | `get`/`put`/`get-int`/`put-int`、`global-get`、`submit(atomic-op)`（`add`、`compare-and-set`、`append`、**新**`try-add`） |
| `event`（永遠） | `player-name`、`deny-message`、**新**`info`、**新**`online` |
| `env`、`registry`、`log`（永遠） | `tick`/`now-millis`/`random`；registry 多了 `damage-type`；`log` 多了 `error` |
| `scheduler`（`scheduler`） | `global`、`for-player`、`at-position`、`cancel` |
| `chat`（`player.message`） | `send`、**新**`tell`（以 uuid，不需 handle）、`broadcast` |
| `hud`（`player.hud`，**新**） | `title`、`action-bar`、`sidebar`/`clear-sidebar`、`bossbar`/`clear-bossbar` |
| `players`（`player.control`，**新**） | `teleport`、`set-game-mode`、`heal`、`kill`、`kick` |
| `inventory`（`inventory`，**新**） | `give`、`take`、`clear`、`open-menu`、`set-slot`、`close-menu` |
| `entities`（`entity.control`，**新**） | `spawn`、`remove`（只能移除自己生成的） |
| `blocks`（`world.write`，**新**） | `set-blocks`（限於呼叫所帶的 cell） |
| `events`（`events.raise`，**新**） | `raise(name, payload, actor) -> decision` |
| `jobs`（manifest 有 `tasks`，**新**） | `submit(id, kind, payload) -> ticket`（給 `async-tasks` 元件，§6） |

manifest（`plugin.toml`）：`id`、`version`、`api = "1"`（相容的 major；host 拒絕不支援的）、`capabilities`、`tasks = "tasks.wasm"`、
`[[subscribe]] event = "..."`（`policy = fail-open|fail-closed`、`bypass-permission`、過濾器 `blocks`／`entities`／`items`／`names`／
`vanilla`／`area`／`kinds`）、`[config]`。事件名稱：`block-break`、`block-place`、`entity-interact`、`entity-attack`、`player-damage`、
`item-use`、`container-click`、`chat`、`command`、`custom`、`observe`、`join`、`leave`、`op-results`。

### 4.3 動作的語意（所有「寫入遊戲」的呼叫）

- 動作在處理器**正常返回時**才提交（trap 或逾時不留下任何東西），進入 host 的 outbox，依（tick、來源玩家、呼叫順序）排序，
  與 region 數量、執行緒數量無關。
- 動作在**下一個序列點**生效；每個動作回傳一個 `ticket`，結果（`applied` 為真表示生效）在下一個 tick 以 `on-results` 送回來源
  所在的 region（插件訂閱 `op-results` 才會送；`on-results` 帶著來源玩家）。
- 動作以 uuid 指名玩家，不需要 handle；玩家離線則 `applied = false`。
- 區塊編輯必須帶呼叫所給的 `cell-handle`（事件或位置工作的 cell），所有座標必須落在那個 cell 內，否則回傳 `edit-error`（不 trap，
  避免一個 bug 讓 fail-closed 的保護處理器變成全面拒絕）。要改別處就 `scheduler.at-position`。
- `entities.remove` 只接受本插件生成的實體（實體身上有 `kiln:plugin` 的 `kiln:owner` 標記）；選單 id、物品標籤、boss bar id
  自動加上插件 id 前綴（`shop:main`、`shop:wand`、`shop:health`），所以兩個插件不會互相冒充。
- 傳送會先讓玩家下車；`give` 放不下的物品掉在腳邊；`take` 要全部都有才扣。

### 4.4 插件引發的可取消事件

`events.raise(name, payload, actor)` 在**同一個情境**（region 或 global）內，同步呼叫其他訂閱 `custom` 事件的插件（依載入順序，
遇第一個 `deny` 停止），回傳結果。名稱自動加上發出者的 id（`arena:join`）；訂閱端的 `names` 過濾由 host 先做。實作：發出者呼叫期間，
host 把同情境中訂閱 `custom` 的其他實例從槽位「借」進它的 store，所以**重入已在呼叫堆疊上的實例根本不可能**（發出者自己不會收到、
從 `on-custom` 處理器裡再發出的事件找不到對象，深度 1），不延後，因此不會繞過保護。失敗政策照常：被呼叫者 trap 或逾時，
`fail-closed` 的訂閱就拒絕。跨 region 沒有同步呼叫——要通知別的 region，就用 `scheduler` 或原子操作。

### 4.5 擁有權與 fail-closed 如何進到新東西

- **玩家／cell／實體／global 命名空間**不變；新範例展示「領地跨 cell 複寫」：放置端寫自己 cell，鄰近 cell 由 `at-position` 工作在擁有它的
  region 內從 global 快照裡的備註複製（`claims`）。
- **逐呼叫預算、strike、降級、速率限制**對新的可取消事件全部適用（`player-damage` 受害者、`container-click` 點擊者各有 token bucket）。
- **fail-closed**：保護類範例（`claims`、`spawn-protection`）每個訂閱都是 fail-closed，trap／逾時／預算用完一律拒絕；
  降級（3 次 strike）後仍拒絕。

## 5. 版本政策（WIT 1.0 凍結）

- `kiln:api@1.0.0` 起適用 semver。`wit/kiln-api.wit` 去掉註解與空白後的 SHA-256 寫在 `crates/kiln-plugin-host/tests/wit_freeze.rs`，
  改動 WIT 的形狀必須同時改雜湊與版本號（只改註解不用）；測試也檢查 WIT 的 package 版本號與 manifest 的 `api` major 一致。
- **只增不破壞的改動 = minor**：新增介面、新增函式到**新**介面、`observed` 新增分支（舊 guest 不會收到：host 依 manifest 的 `kinds` 只送訂閱過的）、
  新增 manifest capability、新增事件種類（host 只送訂閱過的）。**不能**往既有 record 加欄位、往既有函式加參數、往既有 enum 中間插入：
  那些改 canonical ABI，屬 major。
- **破壞性改動 = major**：移除或改簽名的函式、改 record 欄位、改語意。major 需要**兩次改版的淘汰期**：新 major 發佈時，舊 major 仍照常載入
  （host 以套件名稱的 major 分別連結）至少再一個 major。
- manifest 的 `api = "1"`（預設 1）宣告相容的 major；host 拒絕載入它不支援的 major。
- `async-tasks.wit` 依賴 WASI 0.3 的 component-model async，在其穩定前**不在 1.0 的承諾內**：它的形狀雜湊同樣被追蹤，但可在 1.x 內變動
  （changelog 註明）。
- 這個 PR 內從 0.2.0 → 1.0.0 的破壞性改動（遊戲內尚無外部插件，一次做完）：`on-results` 多了 `player` 參數；`entity-event` 與
  `damage-event` 帶 `cell`；`atomic-op`、`observed`、`registry.kind` 多了分支；介面 `chat` 多了 `tell`；`players` 有 `kill`。

## 6. `async-tasks` world（WASI 0.3 的 component-model async）

與遊戲熱路徑分開：另一個 world（`wit/async-tasks.wit`）、另一個引擎設定（開啟 component-model async 與 concurrency）、另一條工作執行緒
（tokio current-thread），一個插件**一個實例**，不綁 region。插件在 `plugin.toml` 以 `tasks = "tasks.wasm"` 指名第二個元件。它只能：

- `http.fetch(request) -> result<response, http-error>`（async；只到 manifest 有 `http:<host>` 的主機，其他回 `denied`；十秒逾時、回應上限 1 MiB；
  底層是 `ureq` 在 blocking pool 上跑）；
- `timers.sleep(ticks)`（async；數**伺服器 tick**，由 B0 餵進去）；
- `storage.get/put/delete`（async；每個插件自己的小型 KV，存在 `kiln/plugins/tasks/<id>/tasks.kv`，key ≤ 256 B、值 ≤ 1 MiB、4096 個 key）。

與遊戲只以訊息溝通：遊戲端 `jobs.submit(id, kind, payload)`（和其他動作一樣在正常返回時才提交）；元件的 `run(job) -> job-result`（async，同時可有多個
在飛）完成後，結果在**下一個 B0** 以 `op-result`（`applied` 與 `value = bytes`）送回來源玩家的 region（沒有玩家就 global）。

- **熱重載**：舊實例連同在飛的工作整個丟棄；每個被打斷的工作以 `on-cancelled`（`reason = reload`、`id` 是插件自己的工作編號）通知新世代，
  新世代自己決定要不要重送（範例 `webhook` 重送）。舊世代的完成結果丟棄。測試：`a_reload_interrupts_jobs_and_the_new_generation_submits_them_again`。
- **strict 模式**：這個 world 不可用（結果取決於牆鐘）——工作立刻失敗，deterministic。
- **預算**：元件用 epoch 中斷每 5 ms 讓出，所以迴圈不停的任務只會吃自己那條執行緒，不會擋 tick；每個實例 64 MiB 記憶體上限。目前沒有逐工作的 CPU 上限。
- **編譯期選項**：cargo feature `async-tasks`（kiln-plugin-host，預設開）。它讓 wasmtime 以 component-model-async 編譯，使每次進入插件的呼叫
  貴約 13 ns（見 §8）；要精簡熱路徑的建置用 `--no-default-features`，有 `tasks` 元件的插件就不會載入（附原因）。
- **誠實的邊界**：Rust 工具鏈沒有 `wasm32-wasip3` target，所以 guest 以 `wasm32-wasip2` 建置，用 wit-bindgen 的 async ABI（`async func`）
  說話；world 不匯入 `wasi:*@0.3` 的介面（沒有 `wasi:sockets`／`wasi:http`），網路只透過 Kiln 自己的 `http`。host 端 WASI p2 以 async linker 提供給 std 需要的部分。

## 7. SDK 與範例

`plugins/sdk`（crate `kiln-plugin-sdk`）：`Plugin` trait（每個 hook 都有預設值）、`export_plugin!`、文字（`Text` 建構器、`IntoSpans`）、
`Verdict::deny("訊息")`、`state` 的型別化存取（`get_i64`/`bump`/`get_str`/`try_add`/`compare_and_set`/`append`…）、`codec::{Writer, Reader}`
（存位置、清單的小編碼器）、`hud`／`players`／`inventory`（`Item`、`Menu` 建構器）／`entities`（`Spawn`）／`blocks`／`events`／`jobs`／`chat::tell`
的薄封裝。`plugins/tasks-sdk`（`kiln-tasks-sdk`）：`Tasks` trait（`async fn run`）、`export_tasks!`、`http::get/post`、`timers::sleep`、`storage`。

範例（`plugins/examples/`；每個都在測試裡跑，`crates/kiln-plugin-host/tests/{api,api_property,async_tasks}.rs` 與 `crates/kiln-sim/tests/plugin_api*.rs`）：

| 範例 | 展示 |
| --- | --- |
| `spawn-protection`（既有） | fail-closed 的重生點保護、cell 範圍領地、`bypass-permission` |
| `claims` | fail-closed 領地：放金磚領地、破壞／放置／開箱／打動物／PvP 都受保護、跨 cell 複寫、每人三塊 |
| `chat-format`（既有） | 聊天改寫、熱重載 |
| `scoreboard-hud` | 逐玩家側欄＋ boss bar＋標題：`player-died`／`player-spawned` 觀察、擊殺數用 global 原子 `add`、`for-player` 週期工作、重載後重排工作 |
| `homes` | `/sethome`、`/home`（暖機 `for-player` 工作，受傷／移動取消）、玩家範圍資料、`teleport` |
| `shop` | 自訂鎖定選單、`try-add` 扣款（同 tick 賽跑也不會超扣）、`give` 帶標籤／名稱／lore／閃光、`item-use` 的魔杖 |
| `npc` | `entities.spawn`／`remove`、實體範圍資料、只能移除自己的 |
| `arena` + `gatekeeper` | 插件間的 `events.raise`（global 與 region 兩種情境）、`set-blocks`（綁 cell）、`teleport`／`clear`／`give`／`set-game-mode`／`kill` |
| `webhook` + `webhook-tasks` | `async-tasks` world：HTTP、計時器、儲存、重載後重送工作 |
| `counter`、`heartbeat`、`ledger`、`petting`（既有） | 觀察、工作與重載、玩家間轉帳（性質測試）、實體資料 |
| `noop` | 量測用：立刻返回的處理器 |

## 8. 量測

VM（12 vCPU，release，20 批取最佳；同機前後對照）。完整數字與方法在 `docs/design-v2-regionized.md` §11.7。

見該節。

## 9. 缺口

- 沒有 `get-block`（§3）；沒有權限節點；沒有每次移動的 hook。
- `player-damage` 的 `amount` 是進 `hurt` 時的原始傷害（護甲、冷卻、魔法減傷之前）；只有「這一下會打中」才會問插件。
- `entity-attack` 只管非玩家實體；打玩家走 `player-damage`。
- 選單只有 `generic_9xN` 的箱子樣式；一般容器的點擊事件在 `vanilla = true` 時送出，拖曳（drag）每個封包送一次，拒絕就重送整個畫面。
- 動作都是序列點套用，不在 region 工作內並行套用（成本低、順序確定；若某插件一個 tick 數千個方塊編輯，會佔序列點的時間）。
- `take` 依物品 key 計，不分標籤。
- `async-tasks`：strict 不可用；沒有逐工作 CPU 上限；一個插件一個實例；不支援 `wasi:*@0.3` 介面。
