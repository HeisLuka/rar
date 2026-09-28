use chaptera_mobile_reader_core::{MobileReaderDocumentV1, PageRenderPlanV1, ViewerDiagnostic};
use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jbyteArray, jlong, jstring};
use serde::Serialize;
use std::collections::BTreeMap;
use std::ptr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Serialize)]
struct MobileOpenReceiptV1 {
    schema_version: &'static str,
    page_count: usize,
    fidelity: String,
    first_page: PageRenderPlanV1,
    diagnostics: Vec<ViewerDiagnostic>,
}

#[derive(Serialize)]
struct MobileSessionReceiptV1 {
    schema_version: &'static str,
    session_id: i64,
    page_count: usize,
    fidelity: String,
    diagnostics: Vec<ViewerDiagnostic>,
}

static NEXT_SESSION_ID: AtomicI64 = AtomicI64::new(1);
static SESSIONS: OnceLock<Mutex<BTreeMap<i64, MobileReaderDocumentV1>>> = OnceLock::new();

fn sessions() -> &'static Mutex<BTreeMap<i64, MobileReaderDocumentV1>> {
    SESSIONS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn open_receipt_json(bytes: &[u8]) -> Result<String, String> {
    let document = MobileReaderDocumentV1::open_default(bytes).map_err(|error| error.to_string())?;
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
        diagnostics: document.diagnostics().to_vec(),
    })
    .map_err(|error| error.to_string())
}

fn open_session_json(bytes: &[u8]) -> Result<String, String> {
    let document = MobileReaderDocumentV1::open_default(bytes).map_err(|error| error.to_string())?;
    if document.page_count() == 0 {
        return Err("PUB opened but contained no readable pages".into());
    }

    let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    if session_id <= 0 {
        return Err("mobile Reader session id space exhausted".into());
    }

    let receipt = MobileSessionReceiptV1 {
        schema_version: "chaptera.mobile-reader-session.v1",
        session_id,
        page_count: document.page_count(),
        fidelity: format!("{:?}", document.fidelity_status()).to_lowercase(),
        diagnostics: document.diagnostics().to_vec(),
    };

    sessions()
        .lock()
        .map_err(|_| "mobile Reader session registry is poisoned".to_owned())?
        .insert(session_id, document);

    serde_json::to_string(&receipt).map_err(|error| error.to_string())
}

fn page_render_plan_json(session_id: i64, page_index: usize) -> Result<String, String> {
    let guard = sessions()
        .lock()
        .map_err(|_| "mobile Reader session registry is poisoned".to_owned())?;
    let document = guard
        .get(&session_id)
        .ok_or_else(|| format!("unknown mobile Reader session {session_id}"))?;
    let plan = document
        .page_render_plan(page_index)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&plan).map_err(|error| error.to_string())
}

fn image_resource_bytes(session_id: i64, resource_key: &str) -> Result<Vec<u8>, String> {
    let guard = sessions()
        .lock()
        .map_err(|_| "mobile Reader session registry is poisoned".to_owned())?;
    let document = guard
        .get(&session_id)
        .ok_or_else(|| format!("unknown mobile Reader session {session_id}"))?;
    document
        .image_resource_bytes_by_key(resource_key)
        .map_err(|error| error.to_string())?
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("image resource {resource_key} is unavailable"))
}

fn java_string(env: JNIEnv<'_>, value: &str) -> jstring {
    env.new_string(value)
        .expect("JNI string allocation")
        .into_raw()
}

fn throw_illegal_state(env: &mut JNIEnv<'_>, message: impl AsRef<str>) {
    let _ = env.throw_new("java/lang/IllegalStateException", message.as_ref());
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
pub extern "system" fn Java_com_chaptera_reader_NativeReader_openSessionJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    bytes: JByteArray<'_>,
) -> jstring {
    let bytes = match env.convert_byte_array(bytes) {
        Ok(bytes) => bytes,
        Err(error) => return java_string(env, &format!("ERR:JNI_BYTES:{error}")),
    };
    match open_session_json(&bytes) {
        Ok(json) => java_string(env, &json),
        Err(error) => java_string(env, &format!("ERR:OPEN:{error}")),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_chaptera_reader_NativeReader_pageRenderPlanJson(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    session_id: jlong,
    page_index: jlong,
) -> jstring {
    if page_index < 0 {
        return java_string(env, "ERR:PAGE:negative page index");
    }
    match page_render_plan_json(session_id, page_index as usize) {
        Ok(json) => java_string(env, &json),
        Err(error) => java_string(env, &format!("ERR:PAGE:{error}")),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_chaptera_reader_NativeReader_imageResourceBytes(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    session_id: jlong,
    resource_key: JString<'_>,
) -> jbyteArray {
    let resource_key: String = match env.get_string(&resource_key) {
        Ok(value) => value.into(),
        Err(error) => {
            throw_illegal_state(&mut env, format!("invalid image resource key: {error}"));
            return ptr::null_mut();
        }
    };
    let bytes = match image_resource_bytes(session_id, &resource_key) {
        Ok(bytes) => bytes,
        Err(error) => {
            throw_illegal_state(&mut env, error);
            return ptr::null_mut();
        }
    };
    match env.byte_array_from_slice(&bytes) {
        Ok(array) => array.into_raw(),
        Err(error) => {
            throw_illegal_state(&mut env, format!("failed to allocate image byte array: {error}"));
            ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_chaptera_reader_NativeReader_closeSession(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    session_id: jlong,
) {
    if let Ok(mut guard) = sessions().lock() {
        guard.remove(&session_id);
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
    fn receipt_schemas_are_stable() {
        assert_eq!("chaptera.mobile-reader-open.v1", "chaptera.mobile-reader-open.v1");
        assert_eq!(
            "chaptera.mobile-reader-session.v1",
            "chaptera.mobile-reader-session.v1"
        );
    }
}
