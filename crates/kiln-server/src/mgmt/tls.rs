//! TLS for the management server: the certificate and key come from a PKCS12 keystore
//! (`JsonRpcSslContextProvider`).

use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::path::Path;
use std::sync::Arc;

/// `JsonRpcSslContextProvider.getKeystorePassword`: the environment variable, else the property.
pub fn keystore_password(property: &str) -> String {
    std::env::var("MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD").unwrap_or_else(|_| property.to_owned())
}

/// The instructions `JsonRpcSslContextProvider.printInstructions` logs when TLS cannot be set up.
pub const INSTRUCTIONS: [&str; 6] = [
    "To use TLS for the management server, please follow these steps:",
    "1. Set the server property 'management-server-tls-enabled' to 'true' to enable TLS",
    "2. Create a keystore file of type PKCS12 containing your server certificate and private key",
    "3. Set the server property 'management-server-tls-keystore' to the path of your keystore file",
    "4. Set the keystore password via the environment variable 'MINECRAFT_MANAGEMENT_TLS_KEYSTORE_PASSWORD', or system property 'management.tls.keystore.password', or server property 'management-server-tls-keystore-password'",
    "5. Restart the server to apply the changes.",
];

/// `JsonRpcSslContextProvider.createFrom`: the first private key of the keystore with its certificate chain.
pub fn server_config(keystore: &str, password: &str) -> Result<Arc<rustls::ServerConfig>, String> {
    if keystore.is_empty() {
        return Err("java.lang.IllegalArgumentException: TLS is enabled but keystore is not configured".into());
    }
    let path = Path::new(keystore);
    if !path.is_file() {
        return Err(format!("java.lang.IllegalArgumentException: Supplied keystore is not a file or does not exist: '{keystore}'"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("java.io.IOException: {e}"))?;
    let store = p12_keystore::KeyStore::from_pkcs12(&bytes, password)
        .map_err(|e| format!("java.io.IOException: keystore password was incorrect or the keystore is damaged ({e})"))?;
    let (key, chain) = store
        .entries()
        .find_map(|(_, entry)| match entry {
            p12_keystore::KeyStoreEntry::PrivateKeyChain(c) => Some((c.key().to_vec(), c.chain().iter().map(|cert| cert.as_der().to_vec()).collect::<Vec<_>>())),
            _ => None,
        })
        .ok_or_else(|| "java.security.UnrecoverableKeyException: no private key in the keystore".to_owned())?;
    let chain: Vec<CertificateDer<'static>> = chain.into_iter().map(CertificateDer::from).collect();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(chain, PrivatePkcs8KeyDer::from(key).into())
        .map_err(|e| format!("javax.net.ssl.SSLException: {e}"))?;
    Ok(Arc::new(config))
}
