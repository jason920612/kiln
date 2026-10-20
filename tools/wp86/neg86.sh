#!/bin/bash
# wp86 negative checks (on the VM, in the wp86-server-ops checkout): undo what one fix does with a one-line edit, run its
# test, require it to FAIL, restore the file.   usage: ~/kt wp86-server-ops bash tools/wp86/neg86.sh [name filter]
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
fails=0
filter="$1"
# negsed "<name>" "<cargo test args>" file 'sed expression'
negsed() {
  local name="$1" tests="$2" f="$3" expr="$4"
  [ -n "$filter" ] && [[ "$name" != *$filter* ]] && return
  sed -i "$expr" "$f"
  if git diff --quiet -- "$f"; then echo "SED CHANGED NOTHING (bad): $name"; fails=$((fails + 1)); git checkout -- "$f"; return; fi
  if [ -n "$DRY" ]; then echo "edit applies: $name"; git checkout -- "$f"; return; fi
  if cargo test --release $tests > /tmp/neg86.out 2>&1; then
    echo "NOT FAILING (bad): $name"; fails=$((fails + 1))
  elif grep -q "^error" /tmp/neg86.out && ! grep -q "^test .* FAILED" /tmp/neg86.out; then echo "BUILD ERROR (bad): $name"; fails=$((fails + 1))
  else echo "fails without the fix (good): $name"; fi
  git checkout -- "$f"
}
OPS="-p kiln-sim --test server_ops"
GLO="-p kiln-sim --test server_globals"
SRV="-p kiln-server --lib"
NET="-p kiln-net --lib"
SIM=crates/kiln-sim/src
negsed "pause when empty" "$OPS pause" $SIM/lib.rs 's/if self.paused_tick(&mut inbox) {/if false \&\& self.paused_tick(\&mut inbox) {/'
negsed "roster" "$OPS roster" $SIM/lib.rs 's/roster.retain(|e| e.conn != conn);/let _ = conn;/'
negsed "default game mode" "$OPS default_game_mode" $SIM/settings.rs 's/self.commands.default_game_mode = Some(mode);//'
negsed "floating kick" "$OPS hovering" $SIM/region.rs 's/if p.above_ground_ticks > max_flying_ticks(p) {/if false {/'
negsed "spawn protection" "$OPS spawn" $SIM/region.rs 's/Some((radius, spawn)) if !p.is_op =>/Some((radius, spawn)) if false =>/'
negsed "player chat packet" "$OPS chat_is_a_player_chat" $SIM/chat_flow.rs 's/        if message.is_system() {/        if true {/'
negsed "signed command chat" "$OPS say_through" $SIM/chat_flow.rs 's/        if message.is_system() {/        if true {/'
negsed "illegal characters" "$OPS bad_chat" $SIM/chat_flow.rs 's/if has_illegal_chars(text) {/if false {/'
negsed "acknowledgement window" "$OPS bad_chat" $SIM/chat.rs 's/if offset < 0 || offset > max {/if false {/'
negsed "checksum" "$OPS bad_chat" $SIM/chat.rs 's/if checksum_byte != 0 \&\& checksum_byte != checksum(&seen) {/if false {/'
negsed "chat spam" "$OPS spam" $SIM/chat_flow.rs 's/if !throttle.under_threshold() \&\& !self.commands.is_op(&p.name) {/if false {/'
negsed "chat hidden" "$OPS hidden" $SIM/lib.rs 's/if self.client.chat_visibility != chat_flow::CHAT_FULL \&\& is_hidden_chat/if false \&\& is_hidden_chat/'
negsed "hidden chat message" "$OPS hidden" $SIM/chat_flow.rs 's/p.send_unfiltered(packets::system_chat(Text::translate("chat.disabled.options"/p.send(packets::system_chat(Text::translate("chat.disabled.options"/'
negsed "enforce secure profile" "$OPS enforced" $SIM/chat.rs 's/if \*enforce {/if false {/'
negsed "rcon output" "$OPS rcon" $SIM/commands.rs 's/std::mem::take(&mut self.commands.rcon_buffer)$/String::new()/'
negsed "announce key" "$OPS announce" $SIM/settings.rs 's/if let Some(announce) = s.announce_advancements {/if let Some(announce) = None::<bool> {/'
negsed "op permission level" "$GLO op_permission" $SIM/commands.rs 's/let level = self.config.server.op_permission_level;/let level = 4;/'
negsed "force gamemode" "$GLO force_gamemode" $SIM/persist.rs 's/Some(m) if !self.config.server.force_gamemode => m,/Some(m) => m,/'
negsed "max world size" "$GLO max_world" $SIM/world_state.rs 's/absolute_max_size: absolute_max_size(),/absolute_max_size: 29_999_984,/'
negsed "entity broadcast range" "$GLO broadcast" $SIM/entities.rs 's/f64::from(BROADCAST_RANGE_PERCENT.load(std::sync::atomic::Ordering::Relaxed).wrapping_mul(chunks \* 16) \/ 100)/f64::from(chunks * 16)/'
negsed "generate-structures" "-p kiln-storage generate_structures" crates/kiln-storage/src/level.rs 's/self.new_generate_structures = generate;/let _ = generate;/'
negsed "status sample" "$NET the_status_names" crates/kiln-net/src/status.rs 's/let sample = (!hidden).then(|| pick_sample(&players, rng));/let sample: Option<Vec<(Uuid, String)>> = (!hidden).then(Vec::new);/'
negsed "status favicon" "$NET the_status_names" crates/kiln-net/src/status.rs 's/if let Some(icon) = self.favicon {/if let Some(icon) = None::<\&str> {/'
negsed "enable-status" "$NET disabled_status" crates/kiln-net/src/lib.rs 's/^    if !shared.info.enable_status.load(Ordering::Relaxed) {/    if false {/'
negsed "legacy ping" "$NET legacy_pings" crates/kiln-net/src/lib.rs 's/if shared.info.enable_status.load(Ordering::Relaxed) \&\& legacy_ping(/if false \&\& legacy_ping(/'
negsed "decode failure kick" "$NET does_not_decode" crates/kiln-net/src/lib.rs 's/return Err(DecodeFailure { packet: format!("serverbound\/{name}") }.into());/let _ = name; bail!("malformed packet");/'
negsed "rate limit" "$NET rate_limit" crates/kiln-net/src/lib.rs 's/if average > rate_limit as f32 {/if false {/'
negsed "login refusal texts" "$NET login_refusals" crates/kiln-net/src/lib.rs 's/"multiplayer.disconnect.server_full"/"multiplayer.disconnect.nope"/'
negsed "rcon login" "$SRV rcon::" crates/kiln-server/src/rcon.rs 's/                    authed = true;/                    authed = false;/'
negsed "query token" "$SRV query::" crates/kiln-server/src/query.rs 's/if c.token != token {/if false {/'
negsed "properties defaults" "$SRV properties::" crates/kiln-server/src/properties.rs 's/let online_mode = r.boolean("online-mode", true);/let online_mode = r.boolean("online-mode", false);/'
negsed "eula" "$SRV eula::" crates/kiln-server/src/eula.rs 's/v.eq_ignore_ascii_case("true")/v == "true"/'
negsed "watchdog text" "$SRV watchdog::" crates/kiln-server/src/watchdog.rs 's/(should be max /(should be at most /'
negsed "log format" "$SRV logging::" crates/kiln-server/src/logging.rs 's/"\[{:02}:{:02}:{:02}\] \[{thread}\/{level}\]: {message}"/"[{:02}:{:02}:{:02}] {thread} {level}: {message}"/'
negsed "log rotation" "$SRV logging::" crates/kiln-server/src/logging.rs 's/std::fs::remove_file(&latest)?;//'
negsed "java properties writer" "$SRV javaprops::" crates/kiln-server/src/javaprops.rs "s/'=' | ':' | '#' | '!' => {/'=' | '#' | '!' => {/"
negsed "command line" "$SRV launch::" crates/kiln-server/src/launch.rs 's/"initSettings" => o.init_settings = true,/"initSettings" => {}/'
negsed "console language jar" "$SRV jar::" crates/kiln-server/src/jar.rs 's/8 => {$/9 => {/'
SIG="-p kiln-sim --test signed_chat"
negsed "signed message verification" "$SIG signed_messages" $SIM/chat.rs 's/self.session.key.verify(Pkcs1v15Sign::new::<sha2::Sha256>(), &digest, &sig.0\[..\]).is_ok()/true/'
negsed "chat session validation" "$SIG not_signed_by" $SIM/chat.rs 's/let ok = services_keys.iter().any(/let ok = true || services_keys.iter().any(/'
negsed "signature cache slots" "$SIG signed_messages" $SIM/chat.rs 's/Some(i) => PackedSignature::Cached(i as i32),/Some(_) => PackedSignature::Full(signature.0.clone()),/'
negsed "older session key" "$SIG older_session" $SIM/chat_flow.rs 's/if old.is_some_and(|s| expires_at_ms < s.expires_at_ms) {/if false {/'
negsed "services public keys document" "-p kiln-net --lib public_keys" crates/kiln-net/src/auth.rs 's/doc.get("playerCertificateKeys")/doc.get("profilePropertyKeys")/'
negsed "log-ips" "$NET log_ips" crates/kiln-net/src/lib.rs 's/if shared.config.log_ips { remote.to_string() }/if true { remote.to_string() }/'
negsed "formatList of one" "-p kiln-command --lib joining_one" crates/kiln-command/src/text.rs 's/if items.len() == 1 {/if false {/'
negsed "stop while paused" "$OPS pause" $SIM/settings.rs 's/if self.commands.stop_requested \&\& !self.stopped {/if false {/'
echo "negative checks failing the wrong way: $fails"
exit $fails
