# Kiln 插件 API（`kiln:api` 1.0）

本文件是 wp51 的成果：先質疑「插件到底需要什麼」，刪掉不需要的，再蓋出最簡單、能涵蓋剩下用途的 API。
執行緒契約（設計文件 §11.2–§11.4）不變：沒有主執行緒、沒有同步的跨 region 呼叫、跨情境只用型別化原子操作、
擁有權命名空間、失敗關閉（fail-closed）。實作現況與量測見 `docs/design-v2-regionized.md` §11.7。

## 1. 方法

1. **質疑需求**：列出真實伺服器靠插件做的事，逐項寫出它真正需要哪些事件與呼叫。
2. **刪**：沒有任何用途需要的、或能用更小的東西組出來的，就不做（寫在 §3，附理由）。
3. **簡化**：剩下的只做一種形狀——事件進來、動作（effect）出去，動作不在處理器裡生效。
4. 才開始蓋（§4 起）。

## 2. 用途與需求

| 用途 | 真正需要的事件 | 真正需要的呼叫 | 決定 |
| --- | --- | --- | --- |
| 保護／領地 | 可取消：`block-break`、`block-place`（對方塊使用物品，含開箱、開門）、`entity-interact`、`entity-attack`、`player-damage` | cell 範圍資料（領地）、`deny-message`、`chat.send` | 保留。`entity-attack` 與 `player-damage` 是新增（動物保護、PvP 區） |
| 經濟 | `join`／`leave`、`op-results`、指令 | global 原子操作（`add`、`compare-and-set`、`append`，新增 `try-add`：餘額不足就不套用）、玩家範圍資料、`chat.send` | 保留；新增 `try-add`，因為「扣款但不得變負」用 CAS 迴圈在多 region 下會一直重試 |
| 聊天／格式 | 可取消 `chat`（取消或改寫） | 玩家名稱、`global-get`（唯讀快照，例如前綴表） | 保留，不變 |
| 權限 | 可取消 `command`、host 端 `bypass-permission` 過濾 | 無 | **部分刪除**：只保留「用指令事件否決」；不做權限節點提供者、不讓插件改 op 等級（見 §3） |
| 小遊戲／競技場 | `death`、`respawn`／`spawn`（觀察）、`player-damage`、`custom`（插件互相詢問）、工作排程 | `teleport`、`set-game-mode`、`give`、`take`、`clear-inventory`、`heal`、`set-blocks`（競技場還原）、HUD | 保留 |
| 自訂物品／GUI／選單 | 可取消 `item-use`（右鍵手上的物品）、可取消 `container-click`（選單與一般容器） | `give`（名稱、說明、插件標籤、模型）、`menus.open`／`set-slot`／`close` | 保留；選單一律鎖定（點擊不會搬動物品），插件只看到點擊 |
| 計分板／HUD | 無（由玩家事件或工作驅動） | `title`、`action-bar`、`sidebar`、`bossbar`（逐玩家，不動伺服器計分板） | 保留；逐玩家虛擬側欄，不碰共享計分板（共享計分板用原版指令與 datapack 就夠） |
| NPC | `entity-interact` | `entities.spawn`／`remove`（只能動自己生成的）、實體範圍資料 | 保留 |
| 反作弊 | `death`／`player-damage`（觀察速率）、玩家狀態查詢 | `player-info`（位置、血量、模式、是否在地面…）、`for-player` 週期工作取樣、`kick` | **部分刪除**：不提供每次移動／封包 hook（見 §3）；改成「週期取樣玩家狀態」 |
| 世界編輯 | 位置工作（`at-position`） | `set-blocks`（批次，限於呼叫所帶的 cell）、`get-block` | 保留；選區、結構檔讀寫在 guest 內自己做 |
| 傳送／家 | 指令、`player-damage`（取消暖機） | `player-info`（目前位置）、`teleport`、`for-player` 工作、玩家範圍資料、`online` 目錄 | 保留 |
| 記錄／稽核 | `block-break`／`block-place` 觀察批次、`join`／`leave` | `log`、`fs.data` | 保留，不變 |
| 跨插件 API（Vault 之類） | `custom` | `events.raise`（同一情境內、同步、可取消） | 保留，用單一個 `custom` 事件做完 |
| 外部世界（Webhook、Discord、資料庫） | 無 | HTTP、計時器、簡單儲存 | 保留，在獨立的 `async-tasks` world（§7），只用訊息接觸遊戲 |

## 3. 刪掉的東西與理由

- **每次方塊更新、每次實體 tick 的 hook**：沒有任何上面的用途需要；成本是每 tick 幾萬次跨邊界。
- **每次移動、封包層級 hook（`packet.observe`）**：反作弊的真實需求是「取樣玩家狀態」，`player-info` 加週期工作就夠，而且不會讓
  guest 看到未驗證的封包。需要時以後加（只增，不破壞）。
- **權限節點系統與改 op 等級**：Kiln 只有 0–4 等級；插件要自己的權限節點，就用 `command` 事件否決加自己的玩家／global 資料。
  讓插件改 op 等級等於讓任何插件取得全伺服器權限。
- **同步的跨 region 呼叫、跨 region 讀取、共享 KV 的 get/put**：契約禁止（v1 已刪）。
- **自訂世界生成、自訂配方、自訂附魔**：用 datapack，插件不需要。
- **直接改共享計分板**：逐玩家虛擬側欄能做 HUD；共享計分板需要的人用 `/scoreboard`（插件可發指令事件之外，不另開 API）。
- **從處理器同步取得其他玩家／實體的狀態**：只有事件帶的玩家、cell、實體可讀；其他人經 `online` 目錄取得名稱與 uuid，再用動作或訊息。
- **「立即生效」的動作**：所有動作都排隊，在序列點（B0／P 結束／指令結束）依確定順序套用。處理器裡的 `teleport` 不會讓同一個處理器看到
  玩家已移動——這是故意的：處理器可以在 trap 時乾淨地放棄，動作不留下部分結果，也不需要跨 region 加鎖。

## 4. 最終 API 表面（WIT `kiln:api@1.0.0`，`wit/kiln-api.wit`）

### 4.1 事件（guest 匯出）

region 實例（每個 region 一個，綁 region 不綁執行緒）：

| 匯出 | 種類 | 說明 |
| --- | --- | --- |
| `on-block-break` / `on-block-place` | 可取消 | 原版時點；`block-place` 涵蓋對方塊使用物品（放置、開箱、倒水） |
| `on-entity-interact` | 可取消 | 右鍵實體；可讀寫該實體的插件資料 |
| `on-entity-attack` | 可取消 | **新**：玩家打實體 |
| `on-player-damage` | 可取消 | **新**：玩家將受傷（原因、攻擊者、傷害量） |
| `on-item-use` | 可取消 | **新**：手上的物品被使用（空氣或方塊）；host 只在物品帶有該插件的標籤、或訂閱的 `items` 過濾符合時才呼叫 |
| `on-container-click` | 可取消 | **新**：選單／容器點擊；插件選單一律鎖定，回傳值只對一般容器有效 |
| `on-chat` / `on-command` | 可取消 | 取消或改寫 |
| `on-custom` | 可取消 | **新**：其他插件用 `events.raise` 發出的事件 |
| `on-observe` | 批次 | 方塊破壞／放置、**新**：`player-died`、`player-spawned`（加入、重生、換世界） |
| `on-task` / `on-results` | B0 | 工作與原子操作／動作結果 |

global 實例（每個插件一個）：`init`、`on-enable`、`on-disable`、`on-join`、`on-leave`、`on-command`（註冊的指令）、`on-task`、
`on-results`、`on-cancelled`、**新**`on-custom`。

### 4.2 呼叫（guest 匯入，依 manifest capability 連結）

| 介面（capability） | 呼叫 |
| --- | --- |
| `state`（永遠） | `get`/`put`/`get-int`/`put-int`、`global-get`、`submit(atomic-op)`（`add`、`compare-and-set`、`append`、**新**`try-add`） |
| `event`（永遠） | `player-name`、`deny-message`、**新**`player-info`、**新**`online` |
| `env`、`registry`、`log`（永遠） | `tick`/`now-millis`/`random`；registry 多了 `damage-type`；`log.info`/`warn`/**新**`error` |
| `scheduler`（`scheduler`） | `global`、`for-player`、`at-position`、`cancel` |
| `chat`（`player.message`） | `send`、`broadcast` |
| `hud`（`player.hud`，**新**） | `title`、`action-bar`、`sidebar`/`clear-sidebar`、`bossbar`/`clear-bossbar` |
| `players`（`player.control`，**新**） | `teleport`、`set-game-mode`、`heal`、`kick` |
| `inventory`（`inventory`，**新**） | `give`、`take`、`clear`、`open-menu`、`set-slot`、`close-menu` |
| `entities`（`entity.control`，**新**） | `spawn`、`remove`（只能移除自己生成的） |
| `world`（`world.write`，**新**） | `set-blocks`（限於呼叫所帶的 cell）、`get-block` |
| `events`（`events.raise`，**新**） | `raise(name, payload, actor) -> decision` |

### 4.3 動作的語意（所有「寫入遊戲」的呼叫）

- 動作在處理器**正常返回時**才提交（trap 或逾時不留下任何東西），進入 host 的 outbox，依（tick、來源玩家、呼叫順序）排序，
  與 region 數量、執行緒數量無關。
- 動作在**下一個序列點**生效（P 階段之後、指令之後、B0 的工作之後）；每個動作回傳一個 `ticket`，結果
  （`applied` 為真表示生效）在下一個 tick 以 `on-results` 送回來源所在的 region（插件訂閱 `op-results` 才會送）。
- 動作以 uuid 指名玩家，不需要 handle；玩家離線則 `applied = false`。
- 區塊編輯必須帶呼叫所給的 `cell-handle`（事件或位置工作的 cell），所有座標必須落在那個 cell 內，否則回傳錯誤（不 trap，
  避免一個 bug 讓 fail-closed 的保護處理器變成全面拒絕）。要改別處就 `scheduler.at-position`。
- `entities.remove` 只接受本插件生成的實體（實體身上有 `kiln:plugin` 標記）；選單 id 與物品標籤自動加上插件 id 前綴
  （`shop:main`、`shop:wand`），所以兩個插件不會互相冒充。

### 4.4 插件引發的可取消事件

`events.raise(name, payload, actor)` 在**同一個情境**（region 或 global）內，同步呼叫其他訂閱 `custom` 事件的插件（依載入順序，
遇第一個 `deny` 停止），回傳結果。名稱自動加上發出者的 id（`arena:join`）。只拒絕重入已在呼叫堆疊上的實例（發出者自己不會收到），
且從 `on-custom` 處理器裡再發出的事件不再派送（深度 1），所以沒有重入問題；不延後，因此不會繞過保護。跨 region 沒有同步呼叫——
要通知別的 region，就用 `scheduler` 或原子操作。

## 5. 版本政策（WIT 1.0 凍結）

- `kiln:api@1.0.0` 起適用 semver。`wit/kiln-api.wit` 的內容雜湊寫在測試裡（`tests/wit_freeze.rs`），改動 WIT 必須同時改雜湊與版本號。
- **只增不破壞的改動 = minor**：新增介面、新增 world export 之外的 import、在 `variant`／`enum` 尾端新增分支（舊 guest 不會收到它不認得的分支：
  host 只送它訂閱過的事件；新增的 `observed` 分支只送給在 manifest 宣告 `api = "1.x"` 且 x 夠大的插件）、新增 manifest capability。
- **破壞性改動 = major**：移除或改簽名的函式、改 record 欄位、改語意。major 需要**兩次改版的淘汰期**：新 major 發佈時，
  舊 major 仍照常載入（host 以 world 名稱後綴 `@1` 分別連結）至少再一個 major。
- manifest 的 `api = "1"`（預設）宣告相容的 major；host 拒絕載入它不支援的 major。
- 未凍結的東西：`async-tasks` world 依賴 WASI 0.3，在 WASI 0.3 發佈穩定前標為 `@since(feature = async-tasks)`，不在 1.0 的承諾內。

## 6. SDK 與範例

`plugins/sdk`（crate `kiln-plugin-sdk`）：`Plugin` trait（每個 hook 都有預設值）、`export_plugin!`、文字建構器（`Text`／`Span`）、
`state` 的型別化存取（`Counter`、`Json`-free 的小編碼器）、`Verdict`、`hud`／`players`／`inventory`／`menus`／`entities`／`world` 的薄封裝、
`Menu` 建構器、`ItemStack` 建構器。範例（`plugins/examples/`，全部在測試裡跑）：

| 範例 | 展示 |
| --- | --- |
| `spawn-protection`（既有） | fail-closed 的重生點保護、cell 範圍領地、`bypass-permission` |
| `claims`（新） | fail-closed 領地：`claim` 指令、玩家擁有領地、破壞／放置／開箱／打動物／PvP 都受保護，陷入錯誤時一律拒絕 |
| `chat-format`（既有，擴充） | 聊天改寫、前綴來自 global 快照 |
| `scoreboard-hud`（新） | 逐玩家側欄＋ bossbar＋動作列：顯示擊殺／死亡／餘額，用 `player-died` 觀察事件與週期工作 |
| `homes`（新） | `/sethome`、`/home`、`/homes`；暖機用 `for-player` 工作，受傷取消；玩家範圍資料；`teleport` |
| `shop`（新） | 自訂選單商店：`menus.open`、`container-click`、`try-add` 扣款、`give` 給物品、`op-results` 完成交易 |
| `arena`（新，測試用） | `custom` 事件、`set-blocks` 還原、`teleport`／`give`／`clear` 的小遊戲流程 |

## 7. `async-tasks` world（WASI 0.3）

與遊戲熱路徑分開：另一個 world、另一個引擎設定（開啟 component-model-async），一個插件**每個 world 一個實例**，
不綁 region。它只能：

- `http.fetch(request) -> response`（async；host 檢查 manifest 的 `http:<host>` 清單，逾時、大小上限）；
- `timers.sleep(ms)`（async；在 strict 模式下以 tick 計）；
- `storage.get/put/delete`（簡單 KV，擁有權命名空間 `async`，每個插件一份，host 負責持久化）。

它與遊戲只以訊息溝通：global 實例經 `submit-job(payload)` 送工作，完成時結果以 `on-results` 風格的 `job-result` 在下一個 tick 回到 global 實例。
熱重載時進行中的工作取消並以 `on-cancelled` 通知（`reason = reload`），新世代可重送。細節與狀態見 `design-v2-regionized.md` §11.7。

## 8. 量測

見 `design-v2-regionized.md` §11.7（呼叫成本、性質測試、strict 模式決定性）。
