use std::{
    cmp, io,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aws_sdk_s3::{Client, error::SdkError, presigning::PresigningConfig, primitives::ByteStream};
use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use http_body::{Body, Frame, SizeHint};
use tokio::io::{AsyncRead, ReadBuf};

use crate::blob_store::{
    BlobProvider, GrantOperation, ProviderCapabilities, ProviderError, ProviderErrorKind,
    ProviderGrant, ProviderGrantRequest, ProviderObjectMetadata,
};

const BODY_CHUNK_BYTES: usize = 64 * 1024;
const MAX_LOCATOR_BYTES: usize = 1024;
const MAX_PRESIGN_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const GENERATION_PREFIX: &str = "etag:";

#[derive(Clone)]
pub struct S3BlobProvider {
    client: Client,
    quarantine_bucket: String,
    private_bucket: String,
    expected_bucket_owner: Option<String>,
}

impl S3BlobProvider {
    pub fn new(
        client: Client,
        quarantine_bucket: impl Into<String>,
        private_bucket: impl Into<String>,
        expected_bucket_owner: Option<String>,
    ) -> Result<Self, ProviderError> {
        let quarantine_bucket = quarantine_bucket.into();
        let private_bucket = private_bucket.into();
        validate_bucket_ref(&quarantine_bucket)?;
        validate_bucket_ref(&private_bucket)?;
        if let Some(owner) = &expected_bucket_owner {
            validate_expected_owner(owner)?;
        }
        Ok(Self {
            client,
            quarantine_bucket,
            private_bucket,
            expected_bucket_owner,
        })
    }

    fn bucket_and_key(&self, object_locator: &str) -> Result<(String, String), ProviderError> {
        validate_locator(object_locator)?;
        let namespace = object_locator
            .split('/')
            .next()
            .ok_or_else(|| provider_other("invalid_object_locator"))?;
        let bucket = match namespace {
            "quarantine" => &self.quarantine_bucket,
            "canonical" | "checkpoints" | "derived" | "exports" => &self.private_bucket,
            _ => return Err(provider_other("unsupported_object_namespace")),
        };
        Ok((bucket.clone(), object_locator.to_owned()))
    }
}

#[async_trait::async_trait]
impl BlobProvider for S3BlobProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            hard_create_only: false,
            hard_exact_or_max_upload_size: false,
            signed_content_type: false,
            strong_head_after_put: true,
        }
    }

    async fn create_immutable(
        &self,
        object_locator: &str,
        expected_byte_len: u64,
        input: Box<dyn AsyncRead + Unpin + Send>,
    ) -> Result<ProviderObjectMetadata, ProviderError> {
        let (bucket, key) = self.bucket_and_key(object_locator)?;
        let content_length =
            i64::try_from(expected_byte_len).map_err(|_| provider_other("object_too_large"))?;
        let body = ByteStream::new(SdkBody::from_body_1_x(ExactAsyncReadBody::new(
            input,
            expected_byte_len,
        )));

        let result = self
            .client
            .put_object()
            .bucket(bucket)
            .key(key)
            .set_expected_bucket_owner(self.expected_bucket_owner.clone())
            .if_none_match("*")
            .content_length(content_length)
            .body(body)
            .send()
            .await;

        let output = result.map_err(|error| provider_sdk_error(&error, ErrorIntent::Create))?;
        let etag = output
            .e_tag()
            .ok_or_else(|| provider_other("s3_put_missing_etag"))?;
        metadata_from_etag(etag, expected_byte_len)
    }

    async fn head_exact(
        &self,
        object_locator: &str,
    ) -> Result<Option<ProviderObjectMetadata>, ProviderError> {
        let (bucket, key) = self.bucket_and_key(object_locator)?;
        let result = self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .set_expected_bucket_owner(self.expected_bucket_owner.clone())
            .send()
            .await;

        let output = match result {
            Ok(output) => output,
            Err(error) => {
                let mapped = provider_sdk_error(&error, ErrorIntent::Head);
                if mapped.kind == ProviderErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(mapped);
            }
        };

        let byte_len = output
            .content_length()
            .ok_or_else(|| provider_other("s3_head_missing_length"))
            .and_then(|value| {
                u64::try_from(value).map_err(|_| provider_other("s3_head_invalid_length"))
            })?;
        let etag = output
            .e_tag()
            .ok_or_else(|| provider_other("s3_head_missing_etag"))?;
        Ok(Some(metadata_from_etag(etag, byte_len)?))
    }

    async fn open_read(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, ProviderError> {
        let (bucket, key) = self.bucket_and_key(object_locator)?;
        let etag = decode_generation(generation)?;
        let result = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .set_expected_bucket_owner(self.expected_bucket_owner.clone())
            .if_match(etag.clone())
            .send()
            .await;

        let output = result.map_err(|error| provider_sdk_error(&error, ErrorIntent::Read))?;
        if let Some(actual) = output.e_tag()
            && actual != etag
        {
            return Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "s3_generation_mismatch",
            ));
        }
        Ok(Box::new(output.body.into_async_read()))
    }

    async fn delete_exact(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<(), ProviderError> {
        let (bucket, key) = self.bucket_and_key(object_locator)?;
        let etag = decode_generation(generation)?;
        self.client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .set_expected_bucket_owner(self.expected_bucket_owner.clone())
            .if_match(etag)
            .send()
            .await
            .map_err(|error| provider_sdk_error(&error, ErrorIntent::Delete))?;
        Ok(())
    }

    async fn issue_grant(
        &self,
        request: &ProviderGrantRequest,
    ) -> Result<ProviderGrant, ProviderError> {
        let (bucket, key) = self.bucket_and_key(&request.object_locator)?;
        let duration = presign_duration(request.expires_at_ms)?;
        let config = PresigningConfig::builder()
            .expires_in(duration)
            .build()
            .map_err(|_| provider_other("s3_presign_config_invalid"))?;

        let opaque_url = match request.operation {
            GrantOperation::UploadCreateOnly => {
                return Err(ProviderError::new(
                    ProviderErrorKind::Other,
                    "s3_upload_grant_requires_signed_headers",
                ));
            }
            GrantOperation::Download => {
                let presigned = self
                    .client
                    .get_object()
                    .bucket(bucket)
                    .key(key)
                    .presigned(config)
                    .await
                    .map_err(|error| provider_sdk_error(&error, ErrorIntent::Grant))?;
                if presigned.headers().next().is_some() {
                    return Err(ProviderError::new(
                        ProviderErrorKind::Other,
                        "s3_download_grant_requires_headers",
                    ));
                }
                presigned.uri().to_string()
            }
        };

        Ok(ProviderGrant {
            opaque_url,
            expires_at_ms: request.expires_at_ms,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum ErrorIntent {
    Create,
    Head,
    Read,
    Delete,
    Grant,
}

fn provider_sdk_error<E>(error: &SdkError<E>, intent: ErrorIntent) -> ProviderError {
    let status = error
        .raw_response()
        .map(|response| response.status().as_u16());
    let transport_ambiguous = matches!(
        error,
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_)
    );
    let kind = match status {
        Some(403) => ProviderErrorKind::AccessDenied,
        Some(404) => ProviderErrorKind::NotFound,
        Some(409) if matches!(intent, ErrorIntent::Create | ErrorIntent::Delete) => {
            ProviderErrorKind::UnknownOutcome
        }
        Some(412) if matches!(intent, ErrorIntent::Create) => ProviderErrorKind::AlreadyExists,
        Some(412)
            if matches!(
                intent,
                ErrorIntent::Head | ErrorIntent::Read | ErrorIntent::Delete
            ) =>
        {
            ProviderErrorKind::NotFound
        }
        Some(code)
            if code >= 500 && matches!(intent, ErrorIntent::Create | ErrorIntent::Delete) =>
        {
            ProviderErrorKind::UnknownOutcome
        }
        None if transport_ambiguous
            && matches!(intent, ErrorIntent::Create | ErrorIntent::Delete) =>
        {
            ProviderErrorKind::UnknownOutcome
        }
        _ => ProviderErrorKind::Other,
    };
    let code = status
        .map(|status| format!("s3_http_{status}"))
        .unwrap_or_else(|| {
            if transport_ambiguous {
                "s3_transport_failure".to_owned()
            } else {
                "s3_sdk_failure".to_owned()
            }
        });
    ProviderError::new(kind, code)
}

fn metadata_from_etag(etag: &str, byte_len: u64) -> Result<ProviderObjectMetadata, ProviderError> {
    let token = encode_generation(etag)?;
    Ok(ProviderObjectMetadata {
        generation: token.clone(),
        byte_len,
        etag: token,
    })
}

fn encode_generation(etag: &str) -> Result<String, ProviderError> {
    if etag.is_empty() {
        return Err(provider_other("s3_empty_etag"));
    }
    let mut token = String::with_capacity(GENERATION_PREFIX.len() + etag.len() * 2);
    token.push_str(GENERATION_PREFIX);
    for byte in etag.bytes() {
        token.push(nibble(byte >> 4));
        token.push(nibble(byte & 0x0f));
    }
    if token.len() > 128 {
        return Err(provider_other("s3_etag_too_long"));
    }
    Ok(token)
}

fn decode_generation(generation: &str) -> Result<String, ProviderError> {
    let encoded = generation
        .strip_prefix(GENERATION_PREFIX)
        .ok_or_else(|| provider_other("invalid_s3_generation"))?;
    if encoded.is_empty() || encoded.len() % 2 != 0 {
        return Err(provider_other("invalid_s3_generation"));
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().as_chunks::<2>().0 {
        let high = from_hex(pair[0]).ok_or_else(|| provider_other("invalid_s3_generation"))?;
        let low = from_hex(pair[1]).ok_or_else(|| provider_other("invalid_s3_generation"))?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).map_err(|_| provider_other("invalid_s3_generation"))
}

fn nibble(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + value - 10) as char,
    }
}

fn from_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn validate_locator(locator: &str) -> Result<(), ProviderError> {
    if locator.is_empty()
        || locator.len() > MAX_LOCATOR_BYTES
        || locator.starts_with('/')
        || locator.ends_with('/')
        || locator.contains('\\')
        || locator.chars().any(char::is_control)
    {
        return Err(provider_other("invalid_object_locator"));
    }
    let mut segments = locator.split('/');
    let namespace = segments
        .next()
        .ok_or_else(|| provider_other("invalid_object_locator"))?;
    if !matches!(
        namespace,
        "quarantine" | "canonical" | "checkpoints" | "derived" | "exports"
    ) {
        return Err(provider_other("unsupported_object_namespace"));
    }
    let rest: Vec<_> = segments.collect();
    if rest.len() < 2
        || rest
            .iter()
            .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err(provider_other("invalid_object_locator"));
    }
    Ok(())
}

fn validate_bucket_ref(bucket: &str) -> Result<(), ProviderError> {
    if bucket.is_empty()
        || bucket.len() > 255
        || bucket.chars().any(char::is_control)
        || bucket.chars().any(char::is_whitespace)
    {
        return Err(provider_other("invalid_s3_bucket"));
    }
    Ok(())
}

fn validate_expected_owner(owner: &str) -> Result<(), ProviderError> {
    if owner.len() != 12 || !owner.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(provider_other("invalid_s3_expected_owner"));
    }
    Ok(())
}

fn presign_duration(expires_at_ms: u64) -> Result<Duration, ProviderError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| provider_other("system_clock_invalid"))?;
    let now_ms =
        u64::try_from(now.as_millis()).map_err(|_| provider_other("system_clock_invalid"))?;
    let ttl_ms = expires_at_ms
        .checked_sub(now_ms)
        .filter(|ttl| *ttl > 0)
        .ok_or_else(|| provider_other("s3_presign_expired"))?;
    let ttl = Duration::from_millis(ttl_ms);
    if ttl > MAX_PRESIGN_TTL {
        return Err(provider_other("s3_presign_ttl_too_long"));
    }
    Ok(ttl)
}

fn provider_other(code: impl Into<String>) -> ProviderError {
    ProviderError::new(ProviderErrorKind::Other, code)
}

struct ExactAsyncReadBody {
    reader: Mutex<Box<dyn AsyncRead + Unpin + Send>>,
    remaining: u64,
    verified_eof: bool,
}

impl ExactAsyncReadBody {
    fn new(reader: Box<dyn AsyncRead + Unpin + Send>, expected_byte_len: u64) -> Self {
        Self {
            reader: Mutex::new(reader),
            remaining: expected_byte_len,
            verified_eof: false,
        }
    }
}

impl Body for ExactAsyncReadBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        if this.verified_eof {
            return Poll::Ready(None);
        }

        if this.remaining == 0 {
            let mut byte = [0_u8; 1];
            let mut read_buf = ReadBuf::new(&mut byte);
            let mut reader = match this.reader.lock() {
                Ok(reader) => reader,
                Err(_) => {
                    return Poll::Ready(Some(Err(io::Error::other(
                        "S3 upload reader lock poisoned",
                    ))));
                }
            };
            return match Pin::new(&mut **reader).poll_read(cx, &mut read_buf) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Err(error)) => Poll::Ready(Some(Err(error))),
                Poll::Ready(Ok(())) if read_buf.filled().is_empty() => {
                    this.verified_eof = true;
                    Poll::Ready(None)
                }
                Poll::Ready(Ok(())) => Poll::Ready(Some(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "S3 upload stream exceeds expected byte length",
                )))),
            };
        }

        let allowed = cmp::min(
            BODY_CHUNK_BYTES,
            usize::try_from(this.remaining).unwrap_or(usize::MAX),
        );
        let mut bytes = vec![0_u8; allowed];
        let mut read_buf = ReadBuf::new(&mut bytes);
        let mut reader = match this.reader.lock() {
            Ok(reader) => reader,
            Err(_) => {
                return Poll::Ready(Some(Err(io::Error::other(
                    "S3 upload reader lock poisoned",
                ))));
            }
        };
        match Pin::new(&mut **reader).poll_read(cx, &mut read_buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Some(Err(error))),
            Poll::Ready(Ok(())) => {
                let count = read_buf.filled().len();
                if count == 0 {
                    return Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "S3 upload stream ended before expected byte length",
                    ))));
                }
                this.remaining -= count as u64;
                bytes.truncate(count);
                Poll::Ready(Some(Ok(Frame::data(Bytes::from(bytes)))))
            }
        }
    }

    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::new();
        hint.set_lower(self.remaining);
        hint
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use aws_smithy_types::byte_stream::ByteStream;

    use super::*;

    #[test]
    fn generation_token_round_trips_quoted_s3_etag() {
        let raw = "\"d41d8cd98f00b204e9800998ecf8427e\"";
        let encoded = encode_generation(raw).unwrap();
        assert!(encoded.starts_with("etag:"));
        assert!(
            encoded
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b':')
        );
        assert_eq!(decode_generation(&encoded).unwrap(), raw);
    }

    #[test]
    fn locator_routes_quarantine_and_private_namespaces() {
        let client = test_client();
        let provider =
            S3BlobProvider::new(client, "chaptera-quarantine", "chaptera-private", None).unwrap();

        let (bucket, key) = provider
            .bucket_and_key("quarantine/tenant-a/upload-1")
            .unwrap();
        assert_eq!(bucket, "chaptera-quarantine");
        assert_eq!(key, "quarantine/tenant-a/upload-1");

        let (bucket, key) = provider
            .bucket_and_key("canonical/tenant-a/blob-1")
            .unwrap();
        assert_eq!(bucket, "chaptera-private");
        assert_eq!(key, "canonical/tenant-a/blob-1");

        assert!(provider.bucket_and_key("../tenant-a/blob-1").is_err());
    }

    #[test]
    fn s3_capabilities_force_direct_upload_size_fallback() {
        let provider = S3BlobProvider::new(
            test_client(),
            "chaptera-quarantine",
            "chaptera-private",
            None,
        )
        .unwrap();
        let capabilities = provider.capabilities();
        assert!(!capabilities.hard_create_only);
        assert!(!capabilities.hard_exact_or_max_upload_size);
        assert!(!capabilities.signed_content_type);
        assert!(capabilities.strong_head_after_put);
    }

    #[tokio::test]
    async fn exact_async_body_streams_without_aggregation_and_checks_length() {
        let body = ExactAsyncReadBody::new(Box::new(Cursor::new(b"chaptera".to_vec())), 8);
        let bytes = ByteStream::new(SdkBody::from_body_1_x(body))
            .collect()
            .await
            .unwrap()
            .into_bytes();
        assert_eq!(&bytes[..], b"chaptera");

        let short = ExactAsyncReadBody::new(Box::new(Cursor::new(b"short".to_vec())), 6);
        assert!(
            ByteStream::new(SdkBody::from_body_1_x(short))
                .collect()
                .await
                .is_err()
        );

        let long = ExactAsyncReadBody::new(Box::new(Cursor::new(b"longer".to_vec())), 5);
        assert!(
            ByteStream::new(SdkBody::from_body_1_x(long))
                .collect()
                .await
                .is_err()
        );
    }

    fn test_client() -> Client {
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .region(aws_sdk_s3::config::Region::new("us-east-1"))
            .build();
        Client::from_conf(config)
    }
}
