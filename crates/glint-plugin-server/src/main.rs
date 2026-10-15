use clap::Parser;
#[derive(Parser)]
#[command(version, about = "Self-hosted Glint plugin repository")]
struct Arguments {
    #[arg(long, env = "GLINT_REGISTRY_LISTEN", default_value = "127.0.0.1:8787")]
    listen: std::net::SocketAddr,
    #[arg(long, env = "GLINT_REGISTRY_DATA", default_value = "./registry-data")]
    data: std::path::PathBuf,
}
#[tokio::main]
async fn main() {
    let args = Arguments::parse();
    let result = async {
        let token = if let Some(path) = std::env::var_os("GLINT_REGISTRY_ADMIN_TOKEN_FILE") {
            use std::io::Read;
            let mut text = String::new();
            std::fs::File::open(path)
                .map_err(|_| "Cannot read management token file")?
                .take(4097)
                .read_to_string(&mut text)
                .map_err(|_| "Cannot read management token file")?;
            if text.len() > 4096 {
                return Err("Management token file too large".into());
            }
            text.trim().to_owned()
        } else {
            std::env::var("GLINT_REGISTRY_ADMIN_TOKEN")
                .map_err(|_| "Set GLINT_REGISTRY_ADMIN_TOKEN or GLINT_REGISTRY_ADMIN_TOKEN_FILE")?
        };
        let repository = glint_plugin_server::Repository::open(&args.data, &token)?;
        let identity = repository.info();
        println!(
            "Source ID: {}\nPublic key: {}",
            identity.source_id, identity.public_key
        );
        let listener = tokio::net::TcpListener::bind(args.listen)
            .await
            .map_err(|_| "Cannot bind registry address")?;
        println!(
            "Listening on {}",
            listener
                .local_addr()
                .map_err(|_| "Cannot read listen address")?
        );
        axum::serve(listener, glint_plugin_server::router(repository))
            .with_graceful_shutdown(shutdown())
            .await
            .map_err(|_| "Registry server failed")?;
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
