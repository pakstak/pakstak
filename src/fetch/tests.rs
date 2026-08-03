use super::RegistryClient;
use super::test_server::{MockResponse, MockServer};
use crate::reference::Reference;
use crate::storage::tests::storage_mutable_in;
use temp_dir::TempDir;

const REPOSITORY: &str = "test/image";
const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const TAR_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar";

#[test]
fn fetch_manifest_and_layer() {
    let layer = tar_layer(&[("hello.txt", b"hello")]);
    let layer_digest = sha256_digest(&layer);
    let manifest = image_manifest(&[(&layer_digest, TAR_LAYER_MEDIA_TYPE)]);
    let manifest_digest = sha256_digest(&manifest);

    let mut server = MockServer::new();
    server.respond(
        MockResponse::ok(manifest)
            .header("Content-Type", MANIFEST_MEDIA_TYPE)
            .header("Docker-Content-Digest", &manifest_digest),
    );
    server.respond(MockResponse::ok(layer).header("Content-Type", "application/octet-stream"));

    let temp_dir = TempDir::new().unwrap();
    let storage = storage_mutable_in(temp_dir.path()).unwrap();
    let reference =
        Reference::parse(&format!("{}/{REPOSITORY}:latest", server.authority())).unwrap();
    let mut client = RegistryClient::new_for_test(
        ureq::tls::Certificate::from_der(server.certificate_der()).to_owned(),
    );

    let fetched = client.fetch_image(&storage, &reference, false).unwrap();

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

fn image_manifest(layers: &[(&str, &str)]) -> Vec<u8> {
    let layers = layers
        .iter()
        .map(|(digest, media_type)| {
            serde_json::json!({
                "digest": digest,
                "mediaType": media_type,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "layers": layers,
    }))
    .unwrap()
}
