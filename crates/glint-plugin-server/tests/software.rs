use glint_plugin_registry::{self as protocol, Client, Source, software as spec};
use glint_plugin_server::Repository;
const TOKEN: &str = "software-test-token-not-a-real-credential";
#[tokio::test]
async fn streaming_upload_publish_verify_download_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let repository = Repository::open(root.path(), TOKEN).unwrap();
    let info = repository.info();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, glint_plugin_server::router(repository))
            .await
            .unwrap();
    });
    let http = reqwest::Client::new();
    let bytes = vec![42u8; 2 * 1024 * 1024 + 17];
    let name = "Glint-API-0.1.3-windows-x64-setup.exe";
    let upload = format!("{url}/v1/apps/glint-api/assets/{name}?target=x86_64-pc-windows-msvc");
    assert_eq!(
        http.post(&upload)
            .body(vec![1])
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let artifact: spec::Artifact = http
        .post(&upload)
        .bearer_auth(TOKEN)
        .body(bytes.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(artifact.size, bytes.len() as u64);
    assert_eq!(artifact.sha256, protocol::digest(&bytes));
    let source = Source {
        name: "test".into(),
        url: url.clone(),
        public_key: info.public_key.clone(),
        allow_http: false,
    };
    let release = spec::Release {
        version: "0.1.3".into(),
        notes: "Update notes".into(),
        published_at: 0,
        artifacts: vec![artifact.clone()],
        yanked: false,
    };
    let remote = source.clone();
    let r = release.clone();
    tokio::task::spawn_blocking(move || {
        let client = Client::new().unwrap();
        client
            .software_publish(&remote, TOKEN, "glint-api", &r)
            .unwrap();
        assert!(
            client
                .software_publish(&remote, TOKEN, "glint-api", &r)
                .unwrap_err()
                .contains("409")
        );
        let feed = client.software_feed(&remote, "glint-api", 1).unwrap();
        assert_eq!(feed.releases[0].version, "0.1.3");
        client
            .software_yank(&remote, TOKEN, "glint-api", "0.1.3", true)
            .unwrap();
        assert!(
            client
                .software_feed(&remote, "glint-api", 2)
                .unwrap()
                .releases[0]
                .yanked
        );
        client
            .software_yank(&remote, TOKEN, "glint-api", "0.1.3", false)
            .unwrap();
        assert!(client.software_feed(&remote, "glint-api", 4).is_err());
    })
    .await
    .unwrap();
    let asset_url = spec::artifact_url(&source, "glint-api", &artifact).unwrap();
    let response = http.get(asset_url).send().await.unwrap();
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains(name)
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), bytes.as_slice());
    let apps: Vec<String> = http
        .get(format!("{url}/v1/apps"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(apps, vec!["glint-api"]);
    let mut bad = release.clone();
    bad.version = "0.1.4".into();
    bad.artifacts[0].sha256 = "a".repeat(64);
    assert!(
        !http
            .post(format!("{url}/v1/apps/glint-api/releases"))
            .bearer_auth(TOKEN)
            .json(&bad)
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    server.abort();
    let reopened = Repository::open(root.path(), TOKEN).unwrap();
    assert_eq!(reopened.info().source_id, info.source_id);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let new_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, glint_plugin_server::router(reopened))
            .await
            .unwrap();
    });
    let signed: protocol::SignedIndex = http
        .get(format!("{new_url}/v1/apps/glint-api/releases"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let feed = spec::verify(&signed, &info.public_key, "glint-api", 3).unwrap();
    assert_eq!(feed.sequence, 3);
    assert_eq!(feed.releases[0].artifacts[0], artifact);
    assert!(!feed.releases[0].yanked);
    server.abort();
}
