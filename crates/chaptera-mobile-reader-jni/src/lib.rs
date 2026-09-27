use chaptera_mobile_reader_core::{
    BoundedLayoutEnvironment, MobileReaderDocumentV1, PageRenderPlanV1,
};
use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::jstring;
use serde::Serialize;

#[derive(Serialize)]
struct MobileOpenReceiptV1 {
    schema_version: &'static str,
    page_count: usize,
    fidelity: String,
    first_page: PageRenderPlanV1,
}

fn environment() -> BoundedLayoutEnvironment {
    BoundedLayoutEnvironment {
        engine_revision: "chaptera-mobile-reader-jni.v1".into(),
        font_set_fingerprint: "android-shell-unresolved".into(),
        resource_fingerprint: "android-shell-local".into(),
    }
}

fn open_receipt_json(bytes: &[u8]) -> Result<String, String> {
    let document = MobileReaderDocumentV1::open(bytes, environment())
        .map_err(|error| error.to_string())?;
    if document.page_count() == 0 {
        return Err("PUB opened but contained no readable pages".into());
    }
    let first_page = document
        .page_render_plan(0)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&MobileOpenReceiptV1 {
        schema_version: "chaptera.mobile-reader-open.v1",
        page_count: document.page_count(),
        fidelity: format!("{:?}", document.fidelity_status()).to_lowercase(),
        first_page,
    })
    .map_err(|error| error.to_string())
}

fn java_string(mut env: JNIEnv<'_>, value: &str) -> jstring {
    env.new_string(value)
        .expect("JNI string allocation")
        .into_raw()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_chaptera_reader_NativeReader_openLocalPubJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    bytes: JByteArray<'_>,
) -> jstring {
    let bytes = match env.convert_byte_array(bytes) {
        Ok(bytes) => bytes,
        Err(error) => return java_string(env, &format!("ERR:JNI_BYTES:{error}")),
    };
    match open_receipt_json(&bytes) {
        Ok(json) => java_string(env, &json),
        Err(error) => java_string(env, &format!("ERR:OPEN:{error}")),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_chaptera_reader_NativeReader_failureDiagnosticJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    bytes: JByteArray<'_>,
) -> jstring {
    let bytes = match env.convert_byte_array(bytes) {
        Ok(bytes) => bytes,
        Err(error) => return java_string(env, &format!("ERR:JNI_BYTES:{error}")),
    };
    match chaptera_mobile_reader_core::local_failure_diagnostic_json(&bytes) {
        Ok(json) => java_string(env, &json),
        Err(error) => java_string(env, &format!("ERR:DIAGNOSTIC:{error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_declares_mobile_boundary_without_platform_storage_claims() {
        let env = environment();
        assert_eq!(env.engine_revision, "chaptera-mobile-reader-jni.v1");
        assert_eq!(env.font_set_fingerprint, "android-shell-unresolved");
    }
}
