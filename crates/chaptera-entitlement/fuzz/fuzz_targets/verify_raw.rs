#![no_main]

use chaptera_entitlement::{
    BuildIdentity, EntitlementVerifier, TrustBundle, TrustedSigner, VerifyContext,
    DEVICE_KEY_ID_LEN,
};
use libfuzzer_sys::fuzz_target;
use p256::ecdsa::SigningKey;
use std::sync::OnceLock;

const TEST_KID: &[u8] = b"fuzz-k1";
static DEVICE_ID: [u8; DEVICE_KEY_ID_LEN] = [0xA5; DEVICE_KEY_ID_LEN];
static VERIFIER: OnceLock<EntitlementVerifier> = OnceLock::new();

fn verifier() -> &'static EntitlementVerifier {
    VERIFIER.get_or_init(|| {
        let signing = SigningKey::from_slice(&[7u8; 32]).expect("fixed fuzz key");
        let public = signing
            .verifying_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec();

        EntitlementVerifier::new(TrustBundle {
            keys: vec![TrustedSigner {
                kid: TEST_KID.to_vec(),
                public_key_sec1: public,
            }],
        })
    })
}

fn context() -> VerifyContext<'static> {
    VerifyContext {
        build: BuildIdentity {
            product_id: "chaptera.editor",
            major: 2,
            released_at: 1_850_000_000,
        },
        expected_subject: Some("user-1"),
        expected_device_key_id: &DEVICE_ID,
    }
}

fuzz_target!(|data: &[u8]| {
    let _ = verifier().verify(data, &context());
});
