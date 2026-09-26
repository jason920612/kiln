//! AES-128-CFB8 stream cipher of an encrypted connection. The shared secret is both key and IV,
//! and each direction keeps its state for the lifetime of the connection. It sits below framing:
//! outgoing bytes are encrypted after compression, incoming bytes decrypted before frame decoding.

use aes::Aes128;
use aes::cipher::KeyIvInit;

pub struct Encryptor(cfb8::Encryptor<Aes128>);
pub struct Decryptor(cfb8::Decryptor<Aes128>);

/// Both directions for a 16-byte shared secret.
pub fn pair(secret: &[u8; 16]) -> (Encryptor, Decryptor) {
    let key = secret.into();
    (Encryptor(cfb8::Encryptor::new(key, key)), Decryptor(cfb8::Decryptor::new(key, key)))
}

impl Encryptor {
    pub fn apply(&mut self, buf: &mut [u8]) {
        self.0.encrypt(buf);
    }
}

impl Decryptor {
    pub fn apply(&mut self, buf: &mut [u8]) {
        self.0.decrypt(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn nist_sp800_38a_cfb8_aes128() {
        // F.3.7 CFB8-AES128.Encrypt uses a separate IV; check the mode itself against it.
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex("000102030405060708090a0b0c0d0e0f");
        let plain = hex("6bc1bee22e409f96e93d7e117393172aae2d");
        let cipher = hex("3b79424c9c0dd436bace9e0ed4586a4f32b9");
        let mut buf = plain.clone();
        cfb8::Encryptor::<Aes128>::new(key[..].try_into().unwrap(), iv[..].try_into().unwrap()).encrypt(&mut buf);
        assert_eq!(buf, cipher);
        cfb8::Decryptor::<Aes128>::new(key[..].try_into().unwrap(), iv[..].try_into().unwrap()).decrypt(&mut buf);
        assert_eq!(buf, plain);
    }

    #[test]
    fn matches_java_aes_cfb8_with_key_as_iv() {
        // Cipher.getInstance("AES/CFB8/NoPadding") with IvParameterSpec(key), as vanilla's Crypt.getCipher.
        let secret: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let (mut enc, mut dec) = pair(&secret);
        let mut buf = b"Kiln encrypts below framing".to_vec();
        enc.apply(&mut buf);
        assert_eq!(buf, hex(JAVA_CIPHERTEXT));
        dec.apply(&mut buf);
        assert_eq!(buf, b"Kiln encrypts below framing");
    }

    const JAVA_CIPHERTEXT: &str = "4195d1142a11ae1148f71aac132a0e0a2e2d195f33db364ba62695";

    #[test]
    fn state_carries_across_calls() {
        let secret = [0x5a; 16];
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 31 % 251) as u8).collect();
        let mut whole = data.clone();
        pair(&secret).0.apply(&mut whole);

        let (mut enc, mut dec) = pair(&secret);
        let mut pieces = data.clone();
        let mut at = 0;
        for n in [1, 7, 16, 100, 3, 873] {
            enc.apply(&mut pieces[at..at + n]);
            at += n;
        }
        assert_eq!(pieces, whole);
        for chunk in pieces.chunks_mut(13) {
            dec.apply(chunk);
        }
        assert_eq!(pieces, data);
    }
}
