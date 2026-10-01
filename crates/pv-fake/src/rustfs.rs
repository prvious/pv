//! The `rustfs` persona: RustFS's `/health` and the S3 operations PV and its tests use, with each
//! request's SigV4 signature checked by `s3s` against the keys PV passes in the environment.
//! Behavior follows recordings of RustFS 1.0.0-beta.7.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::Write;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use hyper::body::Incoming;
use hyper::header::{CONTENT_TYPE, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use s3s::auth::SimpleAuth;
use s3s::crypto::{Checksum, Md5};
use s3s::dto::{
    CreateBucketInput, CreateBucketOutput, DeleteBucketInput, DeleteBucketOutput,
    DeleteObjectInput, DeleteObjectOutput, ETag, GetObjectInput, GetObjectOutput, HeadBucketInput,
    HeadBucketOutput, HeadObjectInput, HeadObjectOutput, PutObjectInput, PutObjectOutput,
    StreamingBlob,
};
use s3s::service::{S3Service, S3ServiceBuilder};
use s3s::{Body, S3, S3Request, S3Response, S3Result, s3_error};
use tokio::net::TcpListener;

use crate::{FakeSettings, accept};

/// The largest object the fake accepts; PV's probe is a few bytes.
const MAX_OBJECT_SIZE: usize = 1 << 20;

/// Handles `rustfs --address <address> --console-address <address> <data dir>`, with the keys in
/// `RUSTFS_ACCESS_KEY` and `RUSTFS_SECRET_KEY`. Returns `None` once serving.
pub(crate) async fn start(argv: &[String], settings: &FakeSettings) -> Result<Option<u8>> {
    let (mut address, mut console_address) = (None, None);
    let mut arguments = argv.iter().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--address" => address = arguments.next(),
            "--console-address" => console_address = arguments.next(),
            // The data directory; objects live in memory.
            _ => {}
        }
    }
    let (Some(address), Some(console_address)) = (address, console_address) else {
        bail!("expected `rustfs --address <address> --console-address <address> <data dir>`");
    };
    let (access_key, mut secret_key) = keys()?;
    if settings.rustfs_reject_credentials {
        secret_key.push_str("-rejected");
    }
    let mut api = S3ServiceBuilder::new(Store::default());
    api.set_auth(SimpleAuth::from_single(access_key, secret_key));
    // PV never uses the console. It knows no keys, so every S3 request there fails, and a runtime
    // started with the two addresses swapped never becomes usable.
    let mut console = S3ServiceBuilder::new(Store::default());
    console.set_auth(SimpleAuth::new());

    for (address, service, health) in [
        (address, api.build(), true),
        (console_address, console.build(), false),
    ] {
        let listener = TcpListener::bind(address.as_str())
            .await
            .with_context(|| format!("binding {address}"))?;
        tokio::spawn(serve(listener, service, health));
    }

    Ok(None)
}

#[expect(
    clippy::disallowed_methods,
    reason = "RustFS reads its keys from the environment PV starts it with"
)]
fn keys() -> Result<(String, String)> {
    let access_key = std::env::var("RUSTFS_ACCESS_KEY").context("RUSTFS_ACCESS_KEY")?;
    let secret_key = std::env::var("RUSTFS_SECRET_KEY").context("RUSTFS_SECRET_KEY")?;

    Ok((access_key, secret_key))
}

/// Hands requests to `s3s`, and with `health`, answers `GET /health` as RustFS's API port does.
async fn serve(listener: TcpListener, service: S3Service, health: bool) {
    loop {
        let stream = accept(&listener).await;
        let service = service.clone();
        tokio::spawn(async move {
            let handler = service_fn(move |request: Request<Incoming>| {
                let service = service.clone();
                async move {
                    if health
                        && request.method() == Method::GET
                        && request.uri().path() == "/health"
                    {
                        return Ok::<_, Infallible>(health_report());
                    }
                    Ok(service
                        .call(request.map(Body::from))
                        .await
                        .unwrap_or_else(|error| {
                            let mut response =
                                Response::new(Body::from(format!("pv-fake: {error}")));
                            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                            response
                        }))
                }
            });
            let _connection_result = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), handler)
                .await;
        });
    }
}

/// RustFS answers with a JSON readiness report; PV checks only the status.
fn health_report() -> Response<Body> {
    let mut response = Response::new(Body::from(r#"{"ready":true}"#.to_owned()));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    response
}

/// Buckets and their objects.
// ponytail: objects live in memory, so they don't survive a restart as RustFS's do. PV creates
// and probes every allocation's bucket on each reconcile, so nothing it does depends on that.
#[derive(Default)]
struct Store(Mutex<BTreeMap<String, BTreeMap<String, Object>>>);

#[derive(Clone)]
struct Object {
    body: Bytes,
    e_tag: ETag,
    last_modified: SystemTime,
}

impl Store {
    fn buckets(&self) -> MutexGuard<'_, BTreeMap<String, BTreeMap<String, Object>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn object(&self, bucket: &str, key: &str) -> S3Result<Object> {
        let buckets = self.buckets();
        let Some(objects) = buckets.get(bucket) else {
            return Err(s3_error!(NoSuchBucket));
        };
        objects
            .get(key)
            .cloned()
            .ok_or_else(|| s3_error!(NoSuchKey))
    }
}

#[async_trait::async_trait]
impl S3 for Store {
    /// RustFS answers 200 for a bucket that already exists, as S3 does in `us-east-1`.
    async fn create_bucket(
        &self,
        request: S3Request<CreateBucketInput>,
    ) -> S3Result<S3Response<CreateBucketOutput>> {
        self.buckets().entry(request.input.bucket).or_default();

        Ok(S3Response::new(CreateBucketOutput::default()))
    }

    async fn head_bucket(
        &self,
        request: S3Request<HeadBucketInput>,
    ) -> S3Result<S3Response<HeadBucketOutput>> {
        if !self.buckets().contains_key(&request.input.bucket) {
            return Err(s3_error!(NoSuchBucket));
        }

        Ok(S3Response::new(HeadBucketOutput::default()))
    }

    async fn delete_bucket(
        &self,
        request: S3Request<DeleteBucketInput>,
    ) -> S3Result<S3Response<DeleteBucketOutput>> {
        let mut buckets = self.buckets();
        match buckets.get(&request.input.bucket) {
            None => return Err(s3_error!(NoSuchBucket)),
            Some(objects) if !objects.is_empty() => return Err(s3_error!(BucketNotEmpty)),
            Some(_objects) => {}
        }
        buckets.remove(&request.input.bucket);

        Ok(S3Response::new(DeleteBucketOutput::default()))
    }

    async fn put_object(
        &self,
        request: S3Request<PutObjectInput>,
    ) -> S3Result<S3Response<PutObjectOutput>> {
        let PutObjectInput {
            bucket, key, body, ..
        } = request.input;
        let mut body = Body::from(body.unwrap_or_else(|| StreamingBlob::from(Bytes::new())));
        let body = body
            .store_all_limited(MAX_OBJECT_SIZE)
            .await
            .map_err(|_error| s3_error!(EntityTooLarge))?;
        let object = Object {
            e_tag: e_tag(&body),
            body,
            last_modified: SystemTime::now(),
        };
        let e_tag = object.e_tag.clone();
        let mut buckets = self.buckets();
        let Some(objects) = buckets.get_mut(&bucket) else {
            return Err(s3_error!(NoSuchBucket));
        };
        objects.insert(key, object);

        Ok(S3Response::new(PutObjectOutput {
            e_tag: Some(e_tag),
            ..PutObjectOutput::default()
        }))
    }

    async fn head_object(
        &self,
        request: S3Request<HeadObjectInput>,
    ) -> S3Result<S3Response<HeadObjectOutput>> {
        let object = self.object(&request.input.bucket, &request.input.key)?;

        Ok(S3Response::new(HeadObjectOutput {
            content_length: Some(content_length(&object.body)?),
            e_tag: Some(object.e_tag),
            last_modified: Some(object.last_modified.into()),
            ..HeadObjectOutput::default()
        }))
    }

    async fn get_object(
        &self,
        request: S3Request<GetObjectInput>,
    ) -> S3Result<S3Response<GetObjectOutput>> {
        let object = self.object(&request.input.bucket, &request.input.key)?;

        Ok(S3Response::new(GetObjectOutput {
            content_length: Some(content_length(&object.body)?),
            e_tag: Some(object.e_tag),
            last_modified: Some(object.last_modified.into()),
            body: Some(StreamingBlob::from(object.body)),
            ..GetObjectOutput::default()
        }))
    }

    /// Deleting a missing object succeeds, as in S3.
    async fn delete_object(
        &self,
        request: S3Request<DeleteObjectInput>,
    ) -> S3Result<S3Response<DeleteObjectOutput>> {
        if let Some(objects) = self.buckets().get_mut(&request.input.bucket) {
            objects.remove(&request.input.key);
        }

        Ok(S3Response::new(DeleteObjectOutput::default()))
    }
}

/// S3's `ETag` for a single-part upload: the hex MD5 of the body.
fn e_tag(body: &[u8]) -> ETag {
    let mut hex = String::with_capacity(32);
    for byte in Md5::checksum(body) {
        let _write_result = write!(hex, "{byte:02x}");
    }

    ETag::Strong(hex)
}

fn content_length(body: &Bytes) -> S3Result<i64> {
    i64::try_from(body.len()).map_err(|_error| s3_error!(InternalError))
}
