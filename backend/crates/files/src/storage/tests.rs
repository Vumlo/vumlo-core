use vumlo_config::{S3Settings, Secret};

use super::*;

fn s3(endpoint: Option<&str>) -> S3Settings {
    S3Settings {
        endpoint: endpoint.map(str::to_owned),
        bucket: "vumlo".to_owned(),
        region: "eu-north-1".to_owned(),
        access_key: Secret::default(),
        secret_key: Secret::default(),
    }
}

#[test]
fn an_s3_client_is_built_without_touching_the_network() {
    // Nothing listens at this address: building must not connect.
    let settings = s3(Some("https://127.0.0.1:1"));
    assert!(Storage::s3(&settings, "access", "secret").is_ok());
    assert!(Storage::s3(&s3(None), "access", "secret").is_ok());
}

#[test]
fn an_s3_endpoint_over_plain_http_is_refused() {
    let settings = s3(Some("http://minio.example.edu"));
    assert!(Storage::s3(&settings, "access", "secret").is_err());
}
