use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
use argon2::{Argon2, Algorithm, Version, Params};
use rand::RngCore;

pub fn generate_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

pub fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    // OWASP Argon2id : m=64MB, t=3, p=1
    let params = Params::new(65536, 3, 1, Some(32)).map_err(|e| e.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

pub fn encrypt_aes_gcm(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher.encrypt(nonce, plaintext).map_err(|e| e.to_string())?;
    Ok((nonce_bytes.to_vec(), ct))
}

pub fn decrypt_aes_gcm(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(nonce);
    cipher.decrypt(nonce, ciphertext).map_err(|_| "Mot de passe incorrect ou fichier corrompu".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ok() {
        let key = [7u8; 32];
        let msg = "Note secrète à chiffrer 🔐".as_bytes();
        let (nonce, ct) = encrypt_aes_gcm(&key, msg).unwrap();
        assert_eq!(decrypt_aes_gcm(&key, &nonce, &ct).unwrap(), msg);
    }

    #[test]
    fn wrong_key_fails() {
        let (nonce, ct) = encrypt_aes_gcm(&[1u8; 32], b"secret").unwrap();
        assert!(decrypt_aes_gcm(&[2u8; 32], &nonce, &ct).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = [9u8; 32];
        let (nonce, mut ct) = encrypt_aes_gcm(&key, "intègre".as_bytes()).unwrap();
        ct[0] ^= 0xFF;
        assert!(decrypt_aes_gcm(&key, &nonce, &ct).is_err());
    }

    #[test]
    fn derive_deterministic_per_salt() {
        let salt = [3u8; 16];
        let k1 = derive_key("motdepasse", &salt).unwrap();
        let k2 = derive_key("motdepasse", &salt).unwrap();
        assert_eq!(k1, k2);
        let k3 = derive_key("motdepasse", &[4u8; 16]).unwrap();
        assert_ne!(k1, k3);
        let k4 = derive_key("autre-mot-de-passe", &salt).unwrap();
        assert_ne!(k1, k4);
    }

    #[test]
    fn salts_are_unique() {
        assert_ne!(generate_salt(), generate_salt());
    }
}
