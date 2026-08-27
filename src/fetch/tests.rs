use super::RegistryClient;
use super::test_server::{MockResponse, MockServer};
use crate::digest::{DigestError, VerificationFailed};
use crate::manifest::FetchedManifest;
use crate::reference::{Reference, Specifier};
use crate::storage::StorageMutable;
use crate::storage::tests::storage_mutable_in;
use temp_dir::TempDir;

const REPOSITORY: &str = "test/image";
const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const TAR_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar";

#[test]
fn manifest_and_layer() {
    let layer = tar_layer(&[("hello.txt", b"hello")]);
    let layer_digest = sha256_digest(&layer);
    let manifest = image_manifest(&[(&layer_digest, TAR_LAYER_MEDIA_TYPE, layer.len() as u64)]);
    let manifest_digest = sha256_digest(&manifest);

    let mut server = MockServer::new();
    server.respond(manifest_response(manifest, &manifest_digest));
    server.respond(MockResponse::ok(layer).header("Content-Type", "application/octet-stream"));

    let temp_dir = TempDir::new().unwrap();
    let storage = storage_mutable_in(temp_dir.path()).unwrap();
    let fetched = fetch(&server, &storage, Specifier::Tag("latest".to_owned())).unwrap();

    assert_eq!(fetched.digest, manifest_digest);
    assert!(storage.is_manifest_saved(&manifest_digest, false).unwrap());
    assert!(storage.get_layer_path(&layer_digest).is_some());

    let manifest_request = server.read_request().unwrap();
    assert_eq!(manifest_request.method, "GET");
    assert_eq!(
        manifest_request.path,
        format!("/v2/{REPOSITORY}/manifests/latest")
    );
    assert_eq!(
        manifest_request.headers.get("accept").map(String::as_str),
        Some(
            "application/vnd.oci.image.index.v1+json, \
application/vnd.docker.distribution.manifest.list.v2+json, \
application/vnd.oci.image.manifest.v1+json, \
application/vnd.docker.distribution.manifest.v2+json"
        )
    );

    let layer_request = server.read_request().unwrap();
    assert_eq!(layer_request.method, "GET");
    assert_eq!(
        layer_request.path,
        format!("/v2/{REPOSITORY}/blobs/{layer_digest}")
    );
    assert_eq!(
        layer_request.headers.get("accept").map(String::as_str),
        Some(TAR_LAYER_MEDIA_TYPE)
    );
    assert!(server.read_request().is_none());
}

#[test]
fn manifest_header_digest_mismatch() {
    manifest_digest_mismatch(Some(sha256_digest(b"different manifest")), None);
}

#[test]
fn pinned_manifest_digest_mismatch() {
    manifest_digest_mismatch(None, Some(sha256_digest(b"requested manifest")));
}

// Checks manifest rejection; arguments select a mismatched response header or requested digest.
fn manifest_digest_mismatch(header_digest: Option<String>, requested_digest: Option<String>) {
    assert!(header_digest.is_some() ^ requested_digest.is_some());
    let manifest = image_manifest(&[]);
    let actual_digest = sha256_digest(&manifest);
    let response_digest = header_digest.as_ref().unwrap_or(&actual_digest);
    let specifier = requested_digest.as_ref().map_or_else(
        || Specifier::Tag("latest".to_owned()),
        |digest| Specifier::Digest(digest.clone()),
    );
    let requested_specifier = specifier.as_typeless_str().to_owned();
    let mismatched_digest = header_digest
        .as_ref()
        .or(requested_digest.as_ref())
        .unwrap();
    let mut server = MockServer::new();
    server.respond(manifest_response(manifest, response_digest));

    let temp_dir = TempDir::new().unwrap();
    let storage = storage_mutable_in(temp_dir.path()).unwrap();
    let error = fetch(&server, &storage, specifier)
        .err()
        .expect("fetch unexpectedly succeeded");

    assert!(matches!(
        error.downcast_ref::<DigestError>(),
        Some(DigestError::VerificationFailed(_))
    ));
    assert!(!storage.is_manifest_saved(&actual_digest, false).unwrap());
    assert!(!storage.is_manifest_saved(mismatched_digest, false).unwrap());
    assert_eq!(
        server.read_request().unwrap().path,
        format!("/v2/{REPOSITORY}/manifests/{requested_specifier}")
    );
    assert!(server.read_request().is_none());
}

#[test]
fn corrupt_layer() {
    let layer = b"not a tar archive".to_vec();
    let layer_digest = sha256_digest(&layer);
    failed_layer(layer, &layer_digest, |error| {
        let error = error.downcast_ref::<std::io::Error>().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::Other);
    });
}

#[test]
fn layer_digest_mismatch() {
    let layer = tar_layer(&[("hello.txt", b"tampered")]);
    let expected_digest = sha256_digest(b"expected layer");
    failed_layer(layer, &expected_digest, |error| {
        assert!(error.downcast_ref::<VerificationFailed>().is_some());
    });
}

fn failed_layer(layer: Vec<u8>, expected_digest: &str, check_error: impl FnOnce(&anyhow::Error)) {
    let manifest = image_manifest(&[(expected_digest, TAR_LAYER_MEDIA_TYPE, layer.len() as u64)]);
    let manifest_digest = sha256_digest(&manifest);
    let server = MockServer::new();
    server.respond(manifest_response(manifest, &manifest_digest));
    server.respond(MockResponse::ok(layer).header("Content-Type", "application/octet-stream"));

    let temp_dir = TempDir::new().unwrap();
    let storage = storage_mutable_in(temp_dir.path()).unwrap();
    let error = fetch(&server, &storage, Specifier::Tag("latest".to_owned()))
        .err()
        .expect("fetch unexpectedly succeeded");

    check_error(&error);
    assert!(storage.get_layer_path(expected_digest).is_none());
    assert!(!storage.is_manifest_saved(&manifest_digest, false).unwrap());
}

#[test]
fn later_layer_failure() {
    let valid_layer = tar_layer(&[("valid.txt", b"valid")]);
    let valid_digest = sha256_digest(&valid_layer);
    let corrupt_layer = b"not a tar archive".to_vec();
    let corrupt_digest = sha256_digest(&corrupt_layer);
    let manifest = image_manifest(&[
        (
            &valid_digest,
            TAR_LAYER_MEDIA_TYPE,
            valid_layer.len() as u64,
        ),
        (
            &corrupt_digest,
            TAR_LAYER_MEDIA_TYPE,
            corrupt_layer.len() as u64,
        ),
    ]);
    let manifest_digest = sha256_digest(&manifest);
    let server = MockServer::new();
    server.respond(manifest_response(manifest, &manifest_digest));
    server
        .respond(MockResponse::ok(valid_layer).header("Content-Type", "application/octet-stream"));
    server.respond(
        MockResponse::ok(corrupt_layer).header("Content-Type", "application/octet-stream"),
    );

    let temp_dir = TempDir::new().unwrap();
    let storage = storage_mutable_in(temp_dir.path()).unwrap();
    let error = fetch(&server, &storage, Specifier::Tag("latest".to_owned()))
        .err()
        .expect("fetch unexpectedly succeeded");

    let error = error.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert!(storage.get_layer_path(&valid_digest).is_some());
    assert!(storage.get_layer_path(&corrupt_digest).is_none());
    assert!(!storage.is_manifest_saved(&manifest_digest, false).unwrap());
}

fn manifest_response(manifest: Vec<u8>, digest: &str) -> MockResponse {
    MockResponse::ok(manifest)
        .header("Content-Type", MANIFEST_MEDIA_TYPE)
        .header("Docker-Content-Digest", digest)
}

fn fetch(
    server: &MockServer,
    storage: &StorageMutable,
    specifier: Specifier,
) -> anyhow::Result<FetchedManifest> {
    let mut reference =
        Reference::parse(&format!("{}/{REPOSITORY}:latest", server.authority())).unwrap();
    reference.specifier = specifier;
    let mut client = RegistryClient::new_for_test(
        ureq::tls::Certificate::from_der(server.certificate_der()).to_owned(),
    );
    client.fetch_image(storage, &reference, false)
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn tar_layer(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut archive = tar::Builder::new(&mut bytes);
        for (path, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, *contents).unwrap();
        }
        archive.finish().unwrap();
    }
    bytes
}

fn image_manifest(layers: &[(&str, &str, u64)]) -> Vec<u8> {
    let layers = layers
        .iter()
        .map(|(digest, media_type, size)| {
            serde_json::json!({
                "digest": digest,
                "mediaType": media_type,
                "size": size,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "layers": layers,
    }))
    .unwrap()
}
