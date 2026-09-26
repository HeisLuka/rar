#![no_main]

use chaptera_entitlement::{
    BuildIdentity, EntitlementVerifier, TrustBundle, TrustedSigner, VerifyContext,
    DEVICE_KEY_ID_LEN, ENTITLEMENT_CONTENT_TYPE,
};
use coset::{iana, CoseSign1Builder, HeaderBuilder, TaggedCborSerializable};
use libfuzzer_sys::fuzz_target;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use std::sync::OnceLock;

const TEST_KID: &[u8] = b"fuzz-k1";
static DEVICE_ID: [u8; DEVICE_KEY_ID_LEN] = [0xA5; DEVICE_KEY_ID_LEN];
static VERIFIER: OnceLock<EntitlementVerifier> = OnceLock::new();

fn signing_key() -> SigningKey {
    SigningKey::from_slice(&[7u8; 32]).expect("fixed fuzz key")
}

fn verifier() -> &'static EntitlementVerifier {
    VERIFIER.get_or_init(|| {
        let signing = signing_key();
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

fn wrap_signed(payload: &[u8]) -> Vec<u8> {
    let signing = signing_key();
    let protected = HeaderBuilder::new()
        .algorithm(iana::Algorithm::ESP256)
        .key_id(TEST_KID.to_vec())
        .content_type(ENTITLEMENT_CONTENT_TYPE.to_owned())
        .build();

    CoseSign1Builder::new()
        .protected(protected)
        .payload(payload.to_vec())
        .create_signature(&[], |tbs| {
            let signature: Signature = signing.sign(tbs);
            signature.to_bytes().to_vec()
        })
        .build()
        .to_tagged_vec()
        .expect("fuzz wrapper COSE")
}

fuzz_target!(|payload: &[u8]| {
    let artifact = wrap_signed(payload);
    let _ = verifier().verify(&artifact, &context());
});
