//! `fetchprofile name|id|entity`: game profiles of players, and lookups by name or id.
//!
//! The profile of an online player, or of a player entity a command selects, is answered at
//! once. Others are looked up through the session service the server was given
//! ([`SimConfig::profile_lookup`](crate::SimConfig), online mode by default), on another thread:
//! the request carries only the name or id asked for, and the answer reaches the simulation as
//! [`ToSim::ProfileLookup`] and is reported to whoever ran the command. Without a lookup
//! (offline mode, no network) a name resolves to the profile the server would give a player of
//! that name (its offline UUID, no textures), and an id nobody online has fails, as vanilla's
//! command does for a profile it cannot find.
//!
//! An offline server that was given a lookup (`KILN_PROFILE_LOOKUP=true`) behaves like vanilla's
//! offline server, which resolves a name through its user cache (the players it has seen, with
//! the ids they logged in with) and asks the session service for the profile of that id, or for
//! the id of a name it has not seen. A player of an offline server is no account there, so the
//! lookup of their id fails like anyone else's; only online mode and servers without a lookup
//! answer for the players they have.

use crate::Sim;
use crate::commands::{CommandSource, PlayerRef};
use kiln_command::Host;
use kiln_command::host::{ProfileProperty, ProfileQuery, ResolvedProfile};
use kiln_command::vanilla::profile::{failure_text, lookup_success_text};
use kiln_link::{LookedUpProfile, Property, ToSim};
use tracing::warn;

/// Lookups on their way at once (a command that would start more fails).
const MAX_PENDING: usize = 16;

fn properties(list: &[Property]) -> Vec<ProfileProperty> {
    list.iter().map(|q| ProfileProperty { name: q.name.clone(), value: q.value.clone(), signature: q.signature.clone() }).collect()
}

impl Sim {
    /// The profile of an online player, by name (case-insensitive, names are unique) or id.
    fn online_profile(&self, query: &ProfileQuery) -> Option<ResolvedProfile> {
        let p = self.players.values().find(|p| match query {
            ProfileQuery::Name(n) => p.name.eq_ignore_ascii_case(n),
            ProfileQuery::Id(id) => p.uuid == *id,
        })?;
        Some(ResolvedProfile { id: p.uuid, name: p.name.clone(), properties: properties(&p.properties) })
    }

    /// `Avatar.getProfile` of a player a command selected.
    pub(crate) fn player_entity_profile(&self, entity: &PlayerRef) -> Option<ResolvedProfile> {
        if entity.entity.is_some() {
            return None;
        }
        let p = self.players.get(&entity.conn)?;
        Some(ResolvedProfile { id: p.uuid, name: p.name.clone(), properties: properties(&p.properties) })
    }

    /// Starts `fetchprofile name|id`.
    pub(crate) fn start_profile_lookup(&mut self, query: ProfileQuery) {
        let asks_everyone = self.config.profile_lookup.is_some() && !self.config.online_mode;
        if !asks_everyone && let Some(profile) = self.online_profile(&query) {
            self.answer_later(&query, Some(profile));
            return;
        }
        if let (Some(lookup), Some(tx)) = (self.config.profile_lookup.clone(), self.config.replies.clone())
            && self.commands.profile_requests.len() < MAX_PENDING
        {
            let request = self.commands.next_profile_request;
            self.commands.next_profile_request += 1;
            self.commands.profile_requests.insert(request, (self.commands.source, query.clone()));
            // The id a name already has here (offline servers: the id the player logged in with).
            let known_id = match &query {
                ProfileQuery::Name(n) if !self.config.online_mode => {
                    self.players.values().find(|p| p.name.eq_ignore_ascii_case(n)).map(|p| p.uuid)
                }
                _ => None,
            };
            let asked = known_id.map_or_else(|| query.clone(), ProfileQuery::Id);
            let spawned = std::thread::Builder::new().name("kiln-profile".into()).spawn(move || {
                let result = match &asked {
                    ProfileQuery::Name(name) => lookup.by_name(name),
                    ProfileQuery::Id(id) => lookup.by_id(*id),
                };
                let result = result.unwrap_or_else(|e| {
                    warn!("profile lookup failed: {e}");
                    None
                });
                let _ = tx.send(ToSim::ProfileLookup { request, result });
            });
            if spawned.is_err() {
                self.commands.profile_requests.remove(&request);
                self.report_profile(self.commands.source, &query, None);
            }
            return;
        }
        // No network: the profile the server would give that name (a server that
        // authenticates players has none for names nobody online has).
        let local = match &query {
            ProfileQuery::Name(name) if !self.config.online_mode => {
                Some(ResolvedProfile { id: crate::commands::offline_uuid(name), name: name.clone(), properties: Vec::new() })
            }
            _ => None,
        };
        self.answer_later(&query, local);
    }

    /// An answer known at once is still delivered on the next tick, like one that had to be
    /// looked up (vanilla answers on the server thread after its lookup task).
    fn answer_later(&mut self, query: &ProfileQuery, profile: Option<ResolvedProfile>) {
        let request = self.commands.next_profile_request;
        self.commands.next_profile_request += 1;
        self.commands.profile_requests.insert(request, (self.commands.source, query.clone()));
        let result = profile.map(|p| LookedUpProfile { id: p.id, name: p.name, properties: p.properties.iter().map(|q| Property { name: q.name.clone(), value: q.value.clone(), signature: q.signature.clone() }).collect() });
        self.commands.profile_results.push((request, result));
    }

    /// Delivers the answers queued by commands of earlier ticks.
    pub(crate) fn deliver_profile_answers(&mut self) {
        for (request, result) in std::mem::take(&mut self.commands.profile_results) {
            self.profile_lookup_finished(request, result);
        }
    }

    /// A lookup finished: the result goes to the source that asked.
    pub(crate) fn profile_lookup_finished(&mut self, request: u64, result: Option<LookedUpProfile>) {
        let Some((source, query)) = self.commands.profile_requests.remove(&request) else { return };
        let profile = result.map(|p| ResolvedProfile { id: p.id, name: p.name, properties: properties(&p.properties) });
        self.report_profile(source, &query, profile.as_ref());
    }

    /// `reportResolvedProfile` or the failure, as feedback to `source`.
    fn report_profile(&mut self, source: CommandSource, query: &ProfileQuery, profile: Option<&ResolvedProfile>) {
        self.as_source(source, |s| match profile {
            Some(p) => s.send_success(lookup_success_text(query, p), false),
            None => s.send_failure(failure_text(query)),
        });
    }
}
