//! Generates proof keys for authorization-code grants. Verifiers must never be logged.

use base64::Engine;
use rand::RngCore;
use sha2::Digest;
use sha2::Sha256;

/// Proof-key values for an OAuth authorization-code exchange.
#[derive(Clone)]
pub struct PkceCodes {
    /// Secret verifier submitted when exchanging the authorization code.
    pub code_verifier: String,
    /// Public S256 challenge sent in the authorization request.
    pub code_challenge: String,
}

/// Generates a fresh PKCE verifier and its S256 challenge.
pub fn generate_pkce() -> PkceCodes {
    let mut bytes = [0u8; 64];
    rand::rng().fill_bytes(&mut bytes);

    // Verifier: URL-safe base64 without padding (43..128 chars)
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

    // Challenge (S256): BASE64URL-ENCODE(SHA256(verifier)) without padding
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

    PkceCodes {
        code_verifier,
        code_challenge,
    }
}
