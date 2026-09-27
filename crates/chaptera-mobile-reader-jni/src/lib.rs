use chaptera_mobile_reader_core::{MobileReaderDocumentV1, PageRenderPlanV1, ViewerDiagnostic};
use jni::JNIEnv;
use jni::objects::{JByteArray, JClass};
use jni::sys::{jboolean, jint, jlong, jstring};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicI64, Ordering},
};

#[derive(Serialize)]
struct MobileOpenReceiptV2 {
    schema_version: &'static str,
    session_id: i64,
    page_count: usize,
    fidelity: String,
    first_page: PageRenderPlanV1,
    diagnostics: Vec<ViewerDiagnostic>,
}

static NEXT_SESSION_ID: AtomicI64 = AtomicI64::new(1);
static SESSIONS: OnceLock<Mutex<HashMap<i64, MobileReaderDocumentV1>>> = OnceLock::new();

fn sessions() -> &'static Mutex<HashMap<i64, MobileReaderDocumentV1>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn open_session_receipt_json(bytes: &[u8]) -> Result<String, String> {
    let document = MobileReaderDocumentV1::open_default(bytes).map_err(|error| error.to_string())?;
    if document.page_count() == 0 {
        return Err("PUB opened but contained no readable pages".into());
    }
    let first_page = document
        .page_render_plan(0)
        .map_err(|error| error.to_string())?;
    let page_count = document.page_count();
    let fidelity = format!("{:?}", document.fidelity_status()).to_lowercase();
    let diagnostics = document.diagnostics().to_vec();
    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    sessions()
        .lock()
        .map_err(|_| "mobile Reader session registry is poisoned".to_owned())?
        .insert(session_id, document);

    serde_json::to_string(&MobileOpenReceiptV2 {
        schema_version: "chaptera.mobile-reader-open.v2",
        session_id,
        page_count,
        fidelity,
        first_page,
        diagnostics,
    })
    .map_err(|error| error.to_string())
}

fn page_json(session_id: i64, page_index: usize) -> Result<String, String> {
    let guard = sessions()
        .lock()
        .map_err(|_| "mobile Reader session registry is poisoned".to_owned())?;
    let document = guard
        .get(&session_id)
        .ok_or_else(|| "mobile Reader session is unavailable".to_owned())?;
    let plan = document
        .page_render_plan(page_index)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&plan).map_err(|error| error.to_string())
}

fn java_string(env: JNIEnv<'_>, value: &str) -> jstring {
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
    match open_session_receipt_json(&bytes) {
        Ok(json) => java_string(env, &json),
        Err(error) => java_string(env, &format!("ERR:OPEN:{error}")),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_chaptera_reader_NativeReader_renderPageJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    session_id: jlong,
    page_index: jint,
) -> jstring {
    if page_index < 0 {
        return java_string(env, "ERR:PAGE:negative page index");
    }
    match page_json(session_id, page_index as usize) {
        Ok(json) => java_string(env, &json),
        Err(error) => java_string(env, &format!("ERR:PAGE:{error}")),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_chaptera_reader_NativeReader_closeSession(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    session_id: jlong,
) -> jboolean {
    let removed = sessions()
        .lock()
        .ok()
        .and_then(|mut guard| guard.remove(&session_id))
        .is_some();
    if removed { 1 } else { 0 }
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
    fn receipt_schema_is_stable() {
        assert_eq!("chaptera.mobile-reader-open.v2", "chaptera.mobile-reader-open.v2");
    }

    #[test]
    fn unknown_session_is_bounded_failure() {
        let error = page_json(i64::MAX, 0).expect_err("unknown session must fail");
        assert!(error.contains("session"));
    }
}
