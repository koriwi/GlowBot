//! Authentication utilities and password encoding

use crate::models::auth::{LoginState, PasswordEncoding};
use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};

/// Password encoder for different Huawei authentication types
pub struct PasswordEncoder;

impl PasswordEncoder {
    /// Encode password based on the device requirements.
    pub fn encode_password(
        username: &str,
        password: &str,
        login_state: &LoginState,
        verification_token: &str,
    ) -> String {
        match login_state.password_encoding() {
            PasswordEncoding::Base64 | PasswordEncoding::Base64AfterChange => {
                Self::encode_base64(password)
            }
            PasswordEncoding::Sha256 | PasswordEncoding::Unknown => {
                Self::encode_sha256(username, password, verification_token)
            }
        }
    }

    fn encode_base64(password: &str) -> String {
        general_purpose::STANDARD.encode(password.as_bytes())
    }

    fn encode_sha256(username: &str, password: &str, verification_token: &str) -> String {
        // Huawei password_type=4 uses two SHA-256 hexdigests wrapped in Base64,
        // with the request verification token salting the outer digest.
        let password_hash = hex::encode(Sha256::digest(password.as_bytes()));
        let password_hash_b64 = general_purpose::STANDARD.encode(password_hash.as_bytes());
        let concentrated = format!("{username}{password_hash_b64}{verification_token}");
        let login_hash = hex::encode(Sha256::digest(concentrated.as_bytes()));
        general_purpose::STANDARD.encode(login_hash.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::auth::LoginState;
    use crate::models::{LockStatus, LoginStatus};

    fn create_test_login_state(password_type: &str) -> LoginState {
        LoginState {
            password_type: password_type.to_string(),
            state: LoginStatus::NotLoggedIn,
            lock_status: LockStatus::Unlocked,
            extern_password_type: "1".to_string(),
            history_login_flag: "0".to_string(),
            guide_modify_pwd_page_flag: "0".to_string(),
            rsa_padding_type: "1".to_string(),
            accounts_number: "1".to_string(),
            wifi_pwd_same_with_web_pwd: "0".to_string(),
            remain_wait_time: "0".to_string(),
            force_skip_guide: "0".to_string(),
            username: "".to_string(),
            first_login: "0".to_string(),
            user_level: "".to_string(),
        }
    }

    #[test]
    fn test_base64_encoding() {
        let login_state = create_test_login_state("0");
        let encoded = PasswordEncoder::encode_password("admin", "admin", &login_state, "token");
        assert_eq!(encoded, "YWRtaW4=");
    }

    #[test]
    fn test_sha256_encoding() {
        let login_state = create_test_login_state("4");
        let encoded = PasswordEncoder::encode_password("admin", "admin", &login_state, "token");
        assert_eq!(
            encoded,
            "OTYxMzMzMjZkNWFkZmY0YmM4MWVhYzNkMjEyNjliOWExZWFmOGQwZjJjMjAwMzMzY2M0ZWEwZjIyZGU2M2NhMg=="
        );
    }

    #[test]
    fn test_unknown_type_defaults_to_sha256() {
        let login_state = create_test_login_state("999");
        let encoded = PasswordEncoder::encode_password("admin", "admin", &login_state, "token");
        assert_eq!(encoded.len(), 88);
    }

    #[test]
    fn test_base64_after_change_encoding() {
        let login_state = create_test_login_state("3");
        let encoded =
            PasswordEncoder::encode_password("admin", "newpassword", &login_state, "token");
        let expected = general_purpose::STANDARD.encode("newpassword".as_bytes());
        assert_eq!(encoded, expected);
    }
}
