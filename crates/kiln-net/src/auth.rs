//! Online-mode authentication: the server's RSA key pair, Minecraft's server hash, and the
//! session server `hasJoined` check (blocking; run it off the async reactor).

use crate::profile::{GameProfile, PropertyJson, check_properties, valid_name};
use anyhow::{Context, Result, bail};
use kiln_link::Property;
use rand_core::{OsRng, RngCore};
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use uuid::Uuid;

/// The server's RSA-1024 key pair, generated at startup like vanilla's.
pub struct ServerKey {
    private: RsaPrivateKey,
    public_der: Vec<u8>,
}

impl ServerKey {
    pub fn generate() -> Result<Self> {
        Self::from_private(RsaPrivateKey::new(&mut OsRng, 1024)?)
    }

    fn from_private(private: RsaPrivateKey) -> Result<Self> {
        let public_der = private.to_public_key().to_public_key_der()?.into_vec();
        Ok(Self { private, public_der })
    }

    /// X.509 SubjectPublicKeyInfo DER, as `PublicKey.getEncoded()` in Java.
    pub fn public_der(&self) -> &[u8] {
        &self.public_der
    }

    /// PKCS#1 v1.5 decryption of an Encryption Response field, with RSA blinding.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        Ok(self.private.decrypt_blinded(&mut OsRng, Pkcs1v15Encrypt, data)?)
    }
}

/// Random bytes for challenges and transaction ids.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0; N];
    OsRng.fill_bytes(&mut b);
    b
}

/// Minecraft's server hash: SHA-1 over server id, shared secret and public key, printed like
/// Java's `new BigInteger(digest).toString(16)` (two's complement, sign, no leading zeros).
pub fn server_hash(server_id: &str, shared_secret: &[u8], public_key_der: &[u8]) -> String {
    debug_assert!(server_id.is_ascii(), "vanilla hashes the server id as ISO-8859-1");
    let digest: [u8; 20] =
        Sha1::new().chain_update(server_id).chain_update(shared_secret).chain_update(public_key_der).finalize().into();
    java_signed_hex(digest)
}

fn java_signed_hex(mut digest: [u8; 20]) -> String {
    let negative = digest[0] & 0x80 != 0;
    if negative {
        // Magnitude of the two's complement value: invert and add one.
        let mut carry = true;
        for b in digest.iter_mut().rev() {
            *b = !*b;
            if carry {
                (*b, carry) = b.overflowing_add(1);
            }
        }
    }
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let digits = match hex.trim_start_matches('0') {
        "" => "0",
        d => d,
    };
    if negative { format!("-{digits}") } else { digits.to_owned() }
}

const DISCOVERY_URL: &str = "https://discovery.minecraftservices.com/minecraft/client";
const FALLBACK_HAS_JOINED: &str = "https://sessionserver.mojang.com/session/minecraft/hasJoined";
const FALLBACK_PROFILE_BY_ID: &str = "https://sessionserver.mojang.com/session/minecraft/profile/{profileId}";
const FALLBACK_PROFILE_BY_NAME: &str = "https://api.mojang.com/users/profiles/minecraft/{name}";
const DISCOVERY_TTL: Duration = Duration::from_secs(3600);
const DISCOVERY_RETRY: Duration = Duration::from_secs(60);
/// Per HTTP request; a login does at most a discovery fetch and a `hasJoined`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound for a whole authentication, for the caller's own timeout.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_RESPONSE: u64 = 256 * 1024;

/// Client for the session server. The `hasJoined` endpoint is resolved through Minecraft
/// services discovery and cached; without a discovered endpoint the well-known one is used.
pub struct SessionService {
    /// Requests go over IPv4 first, as Java prefers it: with a broken IPv6 route a dual-stack
    /// lookup spends most of the timeout on the IPv6 address before trying IPv4.
    ipv4: ureq::Agent,
    /// Fallback for hosts without IPv4 and names without A records.
    any: ureq::Agent,
    discovery_url: String,
    endpoint: Mutex<Option<(String, Instant)>>,
    /// The discovery document, for the profile endpoints.
    document: Mutex<Option<(Option<serde_json::Value>, Instant)>>,
}

impl Default for SessionService {
    fn default() -> Self {
        Self::new(DISCOVERY_URL)
    }
}

impl SessionService {
    pub fn new(discovery_url: &str) -> Self {
        let agent = |family| {
            ureq::Agent::config_builder()
                .timeout_global(Some(REQUEST_TIMEOUT))
                .user_agent(concat!("Kiln/", env!("CARGO_PKG_VERSION")))
                .ip_family(family)
                .build()
                .into()
        };
        Self {
            ipv4: agent(ureq::config::IpFamily::Ipv4Only),
            any: agent(ureq::config::IpFamily::Any),
            discovery_url: discovery_url.to_owned(),
            endpoint: Mutex::new(None),
            document: Mutex::new(None),
        }
    }

    /// A service that skips discovery and asks `has_joined_url` directly.
    #[cfg(test)]
    pub(crate) fn with_endpoint(has_joined_url: &str) -> Self {
        let svc = Self::new("http://127.0.0.1:9/unused-discovery");
        let forever = Instant::now() + Duration::from_secs(86_400);
        *svc.endpoint.lock().unwrap() = Some((has_joined_url.to_owned(), forever));
        svc
    }

    /// Asks the session server whether `name` joined with `server_hash`. `Ok(None)` means it
    /// did not (the client is not authenticated); `Err` means the service could not be asked.
    /// `ip` restricts the check to the client's address (`prevent-proxy-connections`).
    pub fn has_joined(&self, name: &str, server_hash: &str, ip: Option<IpAddr>) -> Result<Option<GameProfile>> {
        let url = self.has_joined_endpoint();
        let ip = ip.map(|ip| ip.to_string());
        let mut query = vec![("username", name), ("serverId", server_hash)];
        query.extend(ip.as_deref().map(|ip| ("ip", ip)));
        let (status, body) = self.get(&url, &query)?;
        if status == 204 || body.trim().is_empty() {
            return Ok(None);
        }
        parse_profile(&body).map(Some)
    }

    /// The profile with `id` and its properties (textures), signed. `Ok(None)`: no such
    /// profile. The request carries the id and nothing else about this server.
    pub fn profile_by_id(&self, id: Uuid) -> Result<Option<GameProfile>> {
        let url = self.endpoint("/discovery/session/endpoints/getProfileById/uri", FALLBACK_PROFILE_BY_ID).replace("{profileId}", &id.simple().to_string());
        match self.get_optional(&url, &[("unsigned", "false")])? {
            Some(body) => parse_profile(&body).map(Some),
            None => Ok(None),
        }
    }

    /// The profile of the account with this name: the name is looked up for its id, then the
    /// profile fetched with its properties. Names that no account can have are not asked about.
    pub fn profile_by_name(&self, name: &str) -> Result<Option<GameProfile>> {
        if name.is_empty() || name.len() > 16 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Ok(None);
        }
        let url = self.endpoint("/discovery/profiles/endpoints/getByName/uri", FALLBACK_PROFILE_BY_NAME).replace("{name}", name);
        let Some(body) = self.get_optional(&url, &[])? else { return Ok(None) };
        let found = parse_profile(&body)?;
        match self.profile_by_id(found.uuid) {
            Ok(Some(full)) => Ok(Some(full)),
            // The profile server may be slow to have it: the name and id are what we know.
            Ok(None) | Err(_) => Ok(Some(found)),
        }
    }

    /// An endpoint of the services discovery document (`{placeholders}` still in it), or
    /// `fallback` when discovery does not answer.
    fn endpoint(&self, pointer: &str, fallback: &str) -> String {
        let mut cache = self.document.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let fresh = cache.as_ref().is_some_and(|(_, until)| now < *until);
        if !fresh {
            let doc = self.get(&self.discovery_url, &[]).ok().and_then(|(_, body)| serde_json::from_str(&body).ok());
            let ttl = if doc.is_some() { DISCOVERY_TTL } else { DISCOVERY_RETRY };
            *cache = Some((doc, now + ttl));
        }
        cache
            .as_ref()
            .and_then(|(doc, _)| doc.as_ref())
            .and_then(|d| d.pointer(pointer))
            .and_then(|v| v.as_str())
            .filter(|u| u.starts_with("https://"))
            .unwrap_or(fallback)
            .to_owned()
    }

    /// GET; `None` for the statuses that mean "no such profile" (204, 404) and empty bodies.
    fn get_optional(&self, url: &str, query: &[(&str, &str)]) -> Result<Option<String>> {
        match self.get(url, query) {
            Ok((204, _)) => Ok(None),
            Ok((_, body)) if body.trim().is_empty() => Ok(None),
            Ok((_, body)) => Ok(Some(body)),
            Err(e) => match e.downcast_ref::<ureq::Error>() {
                Some(ureq::Error::StatusCode(404 | 204)) => Ok(None),
                _ => Err(e),
            },
        }
    }

    fn has_joined_endpoint(&self) -> String {
        let mut cache = self.endpoint.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if let Some((url, until)) = cache.as_ref()
            && now < *until
        {
            return url.clone();
        }
        let (url, ttl) = match self.get(&self.discovery_url, &[]).and_then(|(_, body)| parse_discovery(&body)) {
            Ok(url) => {
                debug!("session endpoint from discovery: {url}");
                (url, DISCOVERY_TTL)
            }
            Err(e) => {
                let url = cache.as_ref().map_or(FALLBACK_HAS_JOINED.to_owned(), |(u, _)| u.clone());
                warn!("services discovery failed ({e:#}); using {url}");
                (url, DISCOVERY_RETRY)
            }
        };
        *cache = Some((url.clone(), now + ttl));
        url
    }

    /// GET returning status and body; error statuses are errors.
    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String)> {
        let call = |agent: &ureq::Agent| query.iter().fold(agent.get(url), |req, (k, v)| req.query(*k, *v)).call();
        let result = match call(&self.ipv4) {
            Err(ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_)) => call(&self.any),
            other => other,
        };
        let mut resp = result.with_context(|| format!("GET {url}"))?;
        let body = resp.body_mut().with_config().limit(MAX_RESPONSE).read_to_string()?;
        Ok((resp.status().as_u16(), body))
    }
}

/// `discovery.session.endpoints.verify.uri` of a discovery document; HTTPS only.
fn parse_discovery(json: &str) -> Result<String> {
    let doc: serde_json::Value = serde_json::from_str(json)?;
    let uri = doc
        .pointer("/discovery/session/endpoints/verify/uri")
        .and_then(|v| v.as_str())
        .context("no session verify endpoint in discovery document")?;
    if !uri.starts_with("https://") {
        bail!("refusing non-HTTPS session endpoint {uri:?}");
    }
    Ok(uri.to_owned())
}

#[derive(Deserialize)]
struct ProfileJson {
    id: String,
    name: String,
    #[serde(default)]
    properties: Vec<PropertyJson>,
}

/// A `hasJoined` response body: `{"id": "<uuid without dashes>", "name": ..., "properties": [...]}`.
fn parse_profile(json: &str) -> Result<GameProfile> {
    let p: ProfileJson = serde_json::from_str(json).context("bad session server response")?;
    let uuid = Uuid::try_parse(&p.id).context("bad profile id")?;
    let properties = p.properties.into_iter().map(Property::from).collect();
    Ok(GameProfile { uuid, name: p.name, properties })
}

/// Checks a session server profile against the name the client logged in with.
pub fn verify_profile(profile: &GameProfile, requested: &str) -> Result<(), &'static str> {
    if !profile.name.eq_ignore_ascii_case(requested) || !valid_name(&profile.name) {
        return Err("session server returned a different name");
    }
    if profile.uuid.is_nil() {
        return Err("session server returned a nil UUID");
    }
    check_properties(&profile.properties)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use kiln_proto::Reader;
    use kiln_proto::packets::login_ext::read_encryption_response;
    use rsa::pkcs8::DecodePrivateKey;

    fn sha1_hex(name: &str) -> String {
        java_signed_hex(Sha1::digest(name.as_bytes()).into())
    }

    #[test]
    fn server_hash_known_values() {
        // The well-known examples of Minecraft's hex digest.
        assert_eq!(sha1_hex("Notch"), "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48");
        assert_eq!(sha1_hex("jeb_"), "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1");
        assert_eq!(sha1_hex("simon"), "88e16a1019277b15d58faf0541e11910eb756f6");
    }

    #[test]
    fn signed_hex_edge_cases() {
        assert_eq!(java_signed_hex([0; 20]), "0");
        assert_eq!(java_signed_hex([0xff; 20]), "-1");
        let mut min = [0; 20];
        min[0] = 0x80;
        assert_eq!(java_signed_hex(min), format!("-8{}", "0".repeat(39)));
    }

    /// PKCS#8 DER of a throwaway RSA-1024 key, used only by these tests.
    const TEST_KEY_PKCS8: &str = include_str!("testdata/rsa1024.pkcs8.hex");

    pub(crate) fn hex(s: &str) -> Vec<u8> {
        let s = s.trim();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// A value from testdata/vanilla_login.txt, written by tools/VanillaLoginVectors.java with
    /// vanilla's codecs and `Crypt` for the test key above.
    pub(crate) fn vanilla_vector(name: &str) -> &'static str {
        include_str!("testdata/vanilla_login.txt")
            .lines()
            .find_map(|l| l.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("no vector {name}"))
    }

    pub(crate) fn test_key() -> ServerKey {
        ServerKey::from_private(RsaPrivateKey::from_pkcs8_der(&hex(TEST_KEY_PKCS8)).unwrap()).unwrap()
    }

    #[test]
    fn public_key_der_matches_java_encoding() {
        assert_eq!(test_key().public_der(), hex(vanilla_vector("public_key")));
    }

    #[test]
    fn decrypts_vanilla_encryption_response() {
        // Vanilla's ServerboundKeyPacket for the secret 00..0f and the challenge 01020304.
        let key = test_key();
        let body = hex(vanilla_vector("key_packet"));
        let resp = read_encryption_response(&mut Reader::new(&body)).unwrap();
        let secret = key.decrypt(resp.shared_secret).unwrap();
        assert_eq!(secret, (0..16).collect::<Vec<u8>>());
        assert_eq!(key.decrypt(resp.challenge).unwrap(), [1, 2, 3, 4]);
        assert_eq!(server_hash("", &secret, key.public_der()), vanilla_vector("server_hash"));
    }

    #[test]
    fn rejects_garbage_ciphertext() {
        let key = test_key();
        assert!(key.decrypt(&[0x42; 128]).is_err());
        assert!(key.decrypt(&[1, 2, 3]).is_err());
    }

    #[test]
    fn generated_key_round_trips() {
        use rsa::RsaPublicKey;
        use rsa::pkcs8::DecodePublicKey;
        let key = ServerKey::generate().unwrap();
        let public = RsaPublicKey::from_public_key_der(key.public_der()).unwrap();
        let ciphertext = public.encrypt(&mut OsRng, Pkcs1v15Encrypt, &[7; 16]).unwrap();
        assert_eq!(key.decrypt(&ciphertext).unwrap(), [7; 16]);
    }

    #[test]
    fn parses_discovery_document() {
        let doc = r#"{"environment":"prod","discovery":{"session":{"endpoints":{
            "join":{"uri":"https://sessionserver.mojang.com/session/minecraft/join"},
            "verify":{"uri":"https://sessionserver.mojang.com/session/minecraft/hasJoined"}}}}}"#;
        assert_eq!(parse_discovery(doc).unwrap(), FALLBACK_HAS_JOINED);
        assert!(parse_discovery(r#"{"discovery":{}}"#).is_err());
        let http = r#"{"discovery":{"session":{"endpoints":{"verify":{"uri":"http://evil.example/x"}}}}}"#;
        assert!(parse_discovery(http).is_err());
    }

    #[test]
    fn parses_and_verifies_has_joined_profile() {
        let body = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[
            {"name":"textures","value":"ewogICJ0aW1lc3RhbXAiIDogMCB9","signature":"c2ln"}],
            "profileActions":[]}"#;
        let p = parse_profile(body).unwrap();
        assert_eq!(p.uuid.to_string(), "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        assert_eq!(p.properties[0].signature.as_deref(), Some("c2ln"));
        assert!(verify_profile(&p, "Notch").is_ok());
        assert!(verify_profile(&p, "notch").is_ok());
        assert!(verify_profile(&p, "jeb_").is_err());
        let nil = GameProfile { uuid: Uuid::nil(), ..p.clone() };
        assert!(verify_profile(&nil, "Notch").is_err());
        assert!(parse_profile(r#"{"id":"zz","name":"Notch"}"#).is_err());
    }

    /// URL of a local server that drops every connection without answering.
    pub(crate) fn dead_url() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || listener.incoming().for_each(drop));
        url
    }

    #[test]
    fn falls_back_when_discovery_is_unreachable() {
        let svc = SessionService::new(&dead_url());
        assert_eq!(svc.has_joined_endpoint(), FALLBACK_HAS_JOINED);
        let (_, until) = svc.endpoint.lock().unwrap().clone().unwrap();
        assert!(until <= Instant::now() + DISCOVERY_RETRY);
    }

    #[test]
    fn keeps_the_discovered_endpoint_when_discovery_fails_later() {
        let svc = SessionService::new(&dead_url());
        let discovered = "https://session.example/hasJoined".to_string();
        *svc.endpoint.lock().unwrap() = Some((discovered.clone(), Instant::now()));
        assert_eq!(svc.has_joined_endpoint(), discovered);
    }
}
