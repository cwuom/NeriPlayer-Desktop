use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_PLAINTEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 3 * 1024 * 1024;
const ENCRYPTED_KIND: &str = "moe.ouom.neriplayer.config.encrypted";
const ITERATIONS: u32 = 600_000;
const AAD: &[u8] = b"moe.ouom.neriplayer.config.encrypted|1|PBKDF2-HMAC-SHA256|600000|AES-256-GCM";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EncryptedConfig {
    kind: String,
    format_version: u32,
    kdf: String,
    iterations: u32,
    cipher: String,
    salt: String,
    iv: String,
    ciphertext: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigBackupError {
    PasswordRequired,
    InvalidPassword,
    InvalidEncryption,
    DecryptionFailed,
    InvalidConfig,
    TooLarge,
}

impl std::fmt::Display for ConfigBackupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PasswordRequired => "CONFIG_PASSWORD_REQUIRED",
            Self::InvalidPassword => "CONFIG_PASSWORD_INVALID",
            Self::InvalidEncryption => "CONFIG_ENCRYPTION_INVALID",
            Self::DecryptionFailed => "CONFIG_DECRYPTION_FAILED",
            Self::InvalidConfig => "CONFIG_FILE_INVALID",
            Self::TooLarge => "CONFIG_FILE_TOO_LARGE",
        })
    }
}

fn validate_password(password: &str) -> Result<(), ConfigBackupError> {
    let length = password.encode_utf16().count();
    if !(1..=1024).contains(&length) {
        return Err(ConfigBackupError::InvalidPassword);
    }
    Ok(())
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, ConfigBackupError> {
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| ConfigBackupError::InvalidEncryption)?;
    if STANDARD.encode(&decoded) != encoded {
        return Err(ConfigBackupError::InvalidEncryption);
    }
    Ok(decoded)
}

fn cipher(password: &str, salt: &[u8]) -> Result<Aes256Gcm, ConfigBackupError> {
    validate_password(password)?;
    let mut key = [0; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), salt, ITERATIONS, &mut key);
    Aes256Gcm::new_from_slice(&key).map_err(|_| ConfigBackupError::InvalidEncryption)
}

fn parse_plaintext(content: &[u8]) -> Result<Value, ConfigBackupError> {
    if content.len() > MAX_PLAINTEXT_BYTES {
        return Err(ConfigBackupError::TooLarge);
    }
    let value: Value =
        serde_json::from_slice(content).map_err(|_| ConfigBackupError::InvalidConfig)?;
    if !value.is_object() {
        return Err(ConfigBackupError::InvalidConfig);
    }
    Ok(value)
}

pub fn decode(content: &[u8], password: Option<&str>) -> Result<Value, ConfigBackupError> {
    if content.len() > MAX_FILE_BYTES {
        return Err(ConfigBackupError::TooLarge);
    }
    let root: Value =
        serde_json::from_slice(content).map_err(|_| ConfigBackupError::InvalidConfig)?;
    if root.get("kind").and_then(Value::as_str) != Some(ENCRYPTED_KIND) {
        return parse_plaintext(content);
    }
    let envelope: EncryptedConfig =
        serde_json::from_value(root).map_err(|_| ConfigBackupError::InvalidEncryption)?;
    if envelope.kind != ENCRYPTED_KIND
        || envelope.format_version != 1
        || envelope.kdf != "PBKDF2-HMAC-SHA256"
        || envelope.iterations != ITERATIONS
        || envelope.cipher != "AES-256-GCM"
    {
        return Err(ConfigBackupError::InvalidEncryption);
    }
    // 在派生密钥前固定算法、轮数和长度，文件不能增加密码计算成本
    let salt = decode_base64(&envelope.salt)?;
    let iv: [u8; 12] = decode_base64(&envelope.iv)?
        .try_into()
        .map_err(|_| ConfigBackupError::InvalidEncryption)?;
    let ciphertext = decode_base64(&envelope.ciphertext)?;
    if salt.len() != 16 || !(16..=MAX_PLAINTEXT_BYTES + 16).contains(&ciphertext.len()) {
        return Err(ConfigBackupError::InvalidEncryption);
    }
    let password = password.ok_or(ConfigBackupError::PasswordRequired)?;
    let plaintext = cipher(password, &salt)?
        .decrypt(
            &Nonce::from(iv),
            Payload {
                msg: &ciphertext,
                aad: AAD,
            },
        )
        .map_err(|_| ConfigBackupError::DecryptionFailed)?;
    parse_plaintext(&plaintext)
}

pub fn encode(config: &Value, password: Option<&str>) -> Result<Vec<u8>, ConfigBackupError> {
    if !config.is_object() {
        return Err(ConfigBackupError::InvalidConfig);
    }
    let plaintext =
        serde_json::to_vec_pretty(config).map_err(|_| ConfigBackupError::InvalidConfig)?;
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(ConfigBackupError::TooLarge);
    }
    let Some(password) = password else {
        return Ok(plaintext);
    };
    validate_password(password)?;
    let mut salt = [0; 16];
    let mut iv = [0; 12];
    OsRng
        .try_fill_bytes(&mut salt)
        .map_err(|_| ConfigBackupError::InvalidEncryption)?;
    OsRng
        .try_fill_bytes(&mut iv)
        .map_err(|_| ConfigBackupError::InvalidEncryption)?;
    let ciphertext = cipher(password, &salt)?
        .encrypt(
            &Nonce::from(iv),
            Payload {
                msg: &plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| ConfigBackupError::InvalidEncryption)?;
    let envelope = EncryptedConfig {
        kind: ENCRYPTED_KIND.into(),
        format_version: 1,
        kdf: "PBKDF2-HMAC-SHA256".into(),
        iterations: ITERATIONS,
        cipher: "AES-256-GCM".into(),
        salt: STANDARD.encode(salt),
        iv: STANDARD.encode(iv),
        ciphertext: STANDARD.encode(ciphertext),
    };
    serde_json::to_vec_pretty(&envelope).map_err(|_| ConfigBackupError::InvalidConfig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    const ANDROID_FIXTURE: &str = r#"{"kind":"moe.ouom.neriplayer.config.encrypted","formatVersion":1,"kdf":"PBKDF2-HMAC-SHA256","iterations":600000,"cipher":"AES-256-GCM","salt":"AAECAwQFBgcICQoLDA0ODw==","iv":"EBESExQVFhcYGRob","ciphertext":"m72RL77kxfGuHlqYa9jgR0S+yXymVQAscd3eN7HmRg00Oj2Xy0vfYZDWbxdV0kpVq3GcihX3hZvyNbyPcmOINGv3Bu+AbXON7+oEEdRdblhZyJv5lB628F1W+4/WIk/DImbunfLBho56wqHYzhM2a201zFMGz01Vx4o2"}"#;

    #[test]
    fn decrypts_android_public_fixture() {
        let value = decode(ANDROID_FIXTURE.as_bytes(), Some("fixture password")).unwrap();
        assert_eq!(value["settings"]["strings"]["audio_quality"], "lossless");
        assert!(value.get("neteaseAuth").is_none());
    }

    #[test]
    fn wrong_password_and_tampering_fail_authentication() {
        assert_eq!(
            decode(ANDROID_FIXTURE.as_bytes(), Some("wrong password")),
            Err(ConfigBackupError::DecryptionFailed)
        );
        let mut value: Value = serde_json::from_str(ANDROID_FIXTURE).unwrap();
        let mut ciphertext = STANDARD
            .decode(value["ciphertext"].as_str().unwrap())
            .unwrap();
        ciphertext[0] ^= 1;
        value["ciphertext"] = STANDARD.encode(ciphertext).into();
        assert_eq!(
            decode(
                &serde_json::to_vec(&value).unwrap(),
                Some("fixture password")
            ),
            Err(ConfigBackupError::DecryptionFailed)
        );
    }

    #[test]
    fn unicode_and_whitespace_passwords_round_trip() {
        let value = serde_json::json!({"kind": "moe.ouom.neriplayer.config", "settings": {"locale": "zh-CN"}});
        for password in [" 密码🔐 e\u{301} ", " ", "fixture password"] {
            let encrypted = encode(&value, Some(password)).unwrap();
            assert_eq!(decode(&encrypted, Some(password)).unwrap(), value);
            assert_eq!(
                decode(&encrypted, None),
                Err(ConfigBackupError::PasswordRequired)
            );
        }
        for password in [String::new(), "🔐".repeat(513)] {
            assert_eq!(
                encode(&value, Some(&password)),
                Err(ConfigBackupError::InvalidPassword)
            );
        }
        assert_eq!(validate_password(&"🔐".repeat(512)), Ok(()));
        assert_eq!(validate_password(&"x".repeat(1024)), Ok(()));
        assert_eq!(
            validate_password(&"x".repeat(1025)),
            Err(ConfigBackupError::InvalidPassword)
        );
        assert_eq!(
            decode(ANDROID_FIXTURE.as_bytes(), Some("")),
            Err(ConfigBackupError::InvalidPassword)
        );
        assert_eq!(decode(&encode(&value, None).unwrap(), None).unwrap(), value);
    }

    #[test]
    fn validates_headers_and_canonical_base64_before_requesting_password() {
        let original: Value = serde_json::from_str(ANDROID_FIXTURE).unwrap();
        for (field, replacement) in [
            ("formatVersion", serde_json::json!(2)),
            ("kdf", serde_json::json!("PBKDF2-HMAC-SHA1")),
            ("iterations", serde_json::json!(600001)),
            ("iterations", serde_json::json!(-1)),
            ("iterations", serde_json::json!(0)),
            ("iterations", serde_json::json!(u32::MAX)),
            ("cipher", serde_json::json!("AES-128-GCM")),
            ("salt", serde_json::json!("AAECAwQFBgcICQoLDA0ODw")),
            ("salt", serde_json::json!("AAECAwQFBgcICQoLDA0ODx==")),
            ("iv", serde_json::json!(" AAESExQVFhcYGRob")),
            ("ciphertext", serde_json::json!("AA==")),
        ] {
            let mut value = original.clone();
            value[field] = replacement;
            assert_eq!(
                decode(&serde_json::to_vec(&value).unwrap(), None),
                Err(ConfigBackupError::InvalidEncryption),
                "{field}"
            );
        }
        for field in ["salt", "iv", "ciphertext", "iterations"] {
            let mut value = original.clone();
            value.as_object_mut().unwrap().remove(field);
            assert_eq!(
                decode(&serde_json::to_vec(&value).unwrap(), None),
                Err(ConfigBackupError::InvalidEncryption)
            );
        }
    }

    #[test]
    fn enforces_file_plaintext_and_ciphertext_size_limits() {
        assert_eq!(
            decode(&vec![b' '; MAX_FILE_BYTES + 1], None),
            Err(ConfigBackupError::TooLarge)
        );
        let oversized = serde_json::json!({"padding": "x".repeat(MAX_PLAINTEXT_BYTES)});
        assert_eq!(encode(&oversized, None), Err(ConfigBackupError::TooLarge));
        assert_eq!(
            decode(&serde_json::to_vec(&oversized).unwrap(), None),
            Err(ConfigBackupError::TooLarge)
        );
        let mut encrypted: Value = serde_json::from_str(ANDROID_FIXTURE).unwrap();
        encrypted["ciphertext"] = STANDARD.encode(vec![0; MAX_PLAINTEXT_BYTES + 17]).into();
        assert_eq!(
            decode(&serde_json::to_vec(&encrypted).unwrap(), None),
            Err(ConfigBackupError::InvalidEncryption)
        );
    }

    #[test]
    fn repeated_exports_use_fresh_salt_and_iv() {
        let value = serde_json::json!({"settings": {}});
        let first: Value =
            serde_json::from_slice(&encode(&value, Some("fixture password")).unwrap()).unwrap();
        let second: Value =
            serde_json::from_slice(&encode(&value, Some("fixture password")).unwrap()).unwrap();
        assert_ne!(first["salt"], second["salt"]);
        assert_ne!(first["iv"], second["iv"]);
        assert_ne!(first["ciphertext"], second["ciphertext"]);
    }
}
