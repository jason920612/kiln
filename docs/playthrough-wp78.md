# wp78:第九次遊玩(晚期內容:下界要塞/堡壘、凋零、終界龍、古城與監守者、神殿與寶藏、突襲、複製機)

分支 `wp78-playthrough9`(從 main `9726b996` 開出)。方法沿用 wp72~wp74:同一個世界(種子 78、noise 產生器)、Kiln 25589 與 vanilla 25595、
`tools/wp78/`(從 wp74 複製:`lock.py` 鎖步、`par.sh` 兩個真客戶端並排、`actor.sh` 重啟單一客戶端)。進行中的紀錄(最後整理):

## 已找到並修好

| # | 問題 | 修正 | 測試 |
|---|---|---|---|
| 1 | 凋零(或它的骷髏)殺死的生物不會長出凋零玫瑰(vanilla `LivingEntity.createWitherRose(getKillCredit())`;griefing 關閉或放不下就掉物品) | `kiln-entity/src/mob/mod.rs::create_wither_rose`、`die` 內在死亡掉落之後呼叫 | `kiln-sim/tests/wither.rs`(3 個) |
| 2 | **力量(Power)、衝擊(Punch)、穿刺(Impaling)完全沒作用**:附魔的條件是 `direct_attacker` 為箭/或 `this` 為水生生物,Kiln 在射出當下用物品情境(沒有實體)判斷,條件永遠不成立。真客戶端:力量 V 的弓打凋零,vanilla 每箭 14.5~22,Kiln 5.8~9.7 | 箭命中時才算(`AbstractArrow.onHitEntity`/`doKnockback`):`kiln-loot` 新增 `ProjectileDamageContext`;`kiln-entity::enchanting::ProjectileHit`;`arrow.rs::hit_living`、`ext_entity/trident.rs::hit_entity` | `kiln-sim/tests/arrow_enchants.rs`(力量 I/V、衝擊) |
