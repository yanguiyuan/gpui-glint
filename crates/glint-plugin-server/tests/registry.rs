use glint_plugin_registry::{
    self as protocol, Client, HostTarget, Package, PluginStore, Publish, Source,
};
use glint_plugin_server::Repository;
const TOKEN: &str = "integration-test-token-not-a-real-credential";
async fn start(root: &std::path::Path) -> (Source, tokio::task::JoinHandle<()>) {
    let repository = Repository::open(root, TOKEN).unwrap();
    let info = repository.info();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = Source {
        name: "test".into(),
        url: format!("http://{}", listener.local_addr().unwrap()),
        public_key: info.public_key,
        allow_http: false,
    };
    let task = tokio::spawn(async move {
        axum::serve(listener, glint_plugin_server::router(repository))
            .await
            .unwrap();
    });
    (source, task)
}
fn publication() -> Publish {
    let plugin=glint_plugin_registry::Package::decode(&serde_json::to_vec(&serde_json::json!({"format":1,"plugin":{"manifest":{"id":"test.plugin","name":"Registry test","version":"1.0.0","api_version":1,"entry":"main.js","capabilities":[],"hooks":["request.before-send"]},"source":"throw new Error('Publishing must not execute code')"}})).unwrap()).unwrap().plugin;
    Publish::new(
        &Package::encode(plugin).unwrap(),
        vec![HostTarget {
            id: "glint-api".into(),
            min_version: "0.1.0".into(),
        }],
        "Test publication".into(),
    )
}
#[tokio::test]
async fn publish_verify_install_withdraw_and_restore() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (source, server) = start(repo.path()).await;
    let body = publication();
    let response = reqwest::Client::new()
        .post(format!("{}/v1/publish", source.url))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let root = home.path().to_owned();
    let source_for_client = source.clone();
    tokio::task::spawn_blocking(move || {
        let client = Client::new().unwrap();
        let store = PluginStore::new(root).unwrap();
        store.add_source(source_for_client.clone()).unwrap();
        let release = client.publish(&source_for_client, TOKEN, &body).unwrap();
        assert!(!release.yanked);
        assert!(
            client
                .publish(&source_for_client, TOKEN, &body)
                .unwrap_err()
                .contains("409")
        );
        let index = store.refresh(&client, &source_for_client).unwrap();
        assert_eq!(index.sequence, 1);
        let installed = store
            .install(
                &client,
                &source_for_client,
                "test.plugin",
                "1.0.0",
                "glint-api",
                "0.1.1",
            )
            .unwrap();
        assert!(
            store
                .install(
                    &client,
                    &source_for_client,
                    "test.plugin",
                    "1.0.0",
                    "database-app",
                    "1.0.0"
                )
                .is_err()
        );
        let plugin = store.load(&installed).unwrap();
        assert!(plugin.identity().contains("::test.plugin::1.0.0"));
        client
            .set_yanked(&source_for_client, TOKEN, "test.plugin", "1.0.0", true)
            .unwrap();
        assert_eq!(
            store.refresh(&client, &source_for_client).unwrap().sequence,
            2
        );
        assert!(
            store
                .install(
                    &client,
                    &source_for_client,
                    "test.plugin",
                    "1.0.0",
                    "glint-api",
                    "0.1.1"
                )
                .is_err()
        );
        assert!(
            store.load(&installed).is_ok(),
            "Withdrawal does not delete previously installed code"
        );
        client
            .set_yanked(&source_for_client, TOKEN, "test.plugin", "1.0.0", false)
            .unwrap();
        assert_eq!(
            store.refresh(&client, &source_for_client).unwrap().sequence,
            3
        );
        store.uninstall("test", "test.plugin", "1.0.0").unwrap();
        assert!(store.installed().unwrap().is_empty());
    })
    .await
    .unwrap();
    server.abort();
    let reopened = Repository::open(repo.path(), TOKEN).unwrap();
    assert_eq!(reopened.info().public_key, source.public_key);
    assert_eq!(
        protocol::verify(&reopened.index().unwrap(), &source.public_key, 3, false)
            .unwrap()
            .sequence,
        3
    );
}
#[tokio::test]
async fn source_identity_prevents_same_id_collisions_and_tampering_is_detected() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (a, one) = start(first.path()).await;
    let (mut b, two) = start(second.path()).await;
    b.name = "second".into();
    let root = home.path().to_owned();
    tokio::task::spawn_blocking(move || {
        let client = Client::new().unwrap();
        let store = PluginStore::new(&root).unwrap();
        store.add_source(a.clone()).unwrap();
        store.add_source(b.clone()).unwrap();
        client.publish(&a, TOKEN, &publication()).unwrap();
        client.publish(&b, TOKEN, &publication()).unwrap();
        let x = store
            .install(&client, &a, "test.plugin", "1.0.0", "glint-api", "0.1.1")
            .unwrap();
        let y = store
            .install(&client, &b, "test.plugin", "1.0.0", "glint-api", "0.1.1")
            .unwrap();
        assert_ne!(
            store.load(&x).unwrap().identity(),
            store.load(&y).unwrap().identity()
        );
        let path = root
            .join("installed")
            .join(a.id().unwrap())
            .join(&x.release.sha256)
            .join("package.glintpkg");
        std::fs::write(path, "tampered").unwrap();
        assert!(store.load(&x).is_err());
        assert!(store.load(&y).is_ok());
        let mut wrong = a.clone();
        wrong.public_key = b.public_key.clone();
        assert!(
            protocol::verify(&client.fetch(&wrong).unwrap(), &wrong.public_key, 0, false).is_err()
        );
    })
    .await
    .unwrap();
    one.abort();
    two.abort();
}

#[tokio::test]
async fn failed_install_preserves_previous_version_and_store_refuses_replayed_index() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (source, server) = start(repo.path()).await;
    let packages = repo.path().join("packages");
    let root = home.path().to_owned();
    tokio::task::spawn_blocking(move || {
        let client = Client::new().unwrap();
        let store = PluginStore::new(&root).unwrap();
        store.add_source(source.clone()).unwrap();
        let old = publication();
        client.publish(&source, TOKEN, &old).unwrap();
        let installed = store
            .install(
                &client,
                &source,
                "test.plugin",
                "1.0.0",
                "glint-api",
                "0.1.1",
            )
            .unwrap();
        let mut plugin = Package::decode(&old.decode().unwrap()).unwrap().plugin;
        plugin.manifest.version = "1.1.0".into();
        let new = Publish::new(
            &Package::encode(plugin).unwrap(),
            old.hosts,
            "Update".into(),
        );
        let release = client.publish(&source, TOKEN, &new).unwrap();
        std::fs::write(
            packages.join(format!("{}.glintpkg", release.sha256)),
            "tampered server package",
        )
        .unwrap();
        assert!(
            store
                .install(
                    &client,
                    &source,
                    "test.plugin",
                    "1.1.0",
                    "glint-api",
                    "0.1.1"
                )
                .is_err()
        );
        assert!(store.load(&installed).is_ok());
        assert_eq!(store.installed().unwrap().len(), 1);
        let latest = store.refresh(&client, &source).unwrap();
        assert_eq!(latest.sequence, 2);
    })
    .await
    .unwrap();
    server.abort();
}
