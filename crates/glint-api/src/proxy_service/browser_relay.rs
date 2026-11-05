use std::{
    io::{ErrorKind, Write},
    net::{Ipv4Addr, TcpListener},
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{
    net::TcpStream,
    sync::{Semaphore, watch},
    task::JoinSet,
};

#[derive(Clone)]
struct Target {
    port: u16,
    stopped: watch::Receiver<bool>,
}

struct BrowserRelay {
    port: u16,
    target: watch::Sender<Target>,
}

// All browser windows started by the app follow the last selected proxy.
static RELAY: Mutex<Option<BrowserRelay>> = Mutex::new(None);

pub(super) fn route(
    directory: &Path,
    port: u16,
    stopped: watch::Receiver<bool>,
) -> Result<u16, String> {
    let mut relay = RELAY.lock().map_err(|_| "浏览器代理入口不可用")?;
    let target = Target { port, stopped };
    if let Some(relay) = relay.as_ref() {
        relay.target.send_if_modified(|previous| {
            if previous.port == target.port && previous.stopped.same_channel(&target.stopped) {
                return false;
            }
            *previous = target;
            true
        });
        return Ok(relay.port);
    }

    let runtime = super::runtime()?;
    let listener = bind(directory)?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (target, receiver) = watch::channel(target);
    runtime.spawn(serve(listener, receiver));
    *relay = Some(BrowserRelay { port, target });
    Ok(port)
}

fn bind(directory: &Path) -> Result<TcpListener, String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let marker = directory.join("browser-proxy-port");
    let saved = match std::fs::read_to_string(&marker) {
        Ok(port) => Some(
            port.trim()
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or("浏览器代理端口记录无效")?,
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, saved.unwrap_or(0)))
        .map_err(|e| format!("无法监听浏览器代理入口：{e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    if saved.is_none() {
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)
            .map_err(|e| format!("无法保存浏览器代理端口：{e}"))?;
        writeln!(marker, "{port}").map_err(|e| format!("无法保存浏览器代理端口：{e}"))?;
    }
    Ok(listener)
}

async fn serve(listener: TcpListener, target: watch::Receiver<Target>) {
    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
        return;
    };
    let limits = Arc::new(Semaphore::new(512));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((mut browser, _)) = accepted else { break; };
                let Ok(permit) = limits.clone().try_acquire_owned() else { continue; };
                let mut route = target.clone();
                let Target { port, mut stopped } = route.borrow_and_update().clone();
                if *stopped.borrow_and_update() {
                    continue;
                }
                connections.spawn(async move {
                    let _permit = permit;
                    let relay = async {
                        let mut upstream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
                        tokio::io::copy_bidirectional(&mut browser, &mut upstream).await
                    };
                    tokio::select! {
                        biased;
                        _ = stopped.changed() => {},
                        _ = route.changed() => {},
                        _ = relay => {},
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn origin(value: u8) -> u16 {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_u8(value).await.unwrap();
            let _ = socket.read_u8().await;
        });
        port
    }

    #[test]
    fn browser_endpoint_survives_listener_changes_and_disconnects_stale_connections() {
        let directory = tempfile::tempdir().unwrap();
        let listener = bind(directory.path()).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let listener = bind(directory.path()).unwrap();
        assert_eq!(listener.local_addr().unwrap().port(), port);

        super::super::runtime().unwrap().block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let (_first, stopped) = watch::channel(false);
                let (target, receiver) = watch::channel(Target {
                    port: origin(1).await,
                    stopped,
                });
                let relay = tokio::spawn(serve(listener, receiver));

                let mut browser = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                    .await
                    .unwrap();
                assert_eq!(browser.read_u8().await.unwrap(), 1);

                let (second, stopped) = watch::channel(false);
                target.send_replace(Target {
                    port: origin(2).await,
                    stopped,
                });
                assert!(browser.read_u8().await.is_err());

                let mut browser = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                    .await
                    .unwrap();
                assert_eq!(browser.read_u8().await.unwrap(), 2);
                second.send(true).unwrap();
                assert!(browser.read_u8().await.is_err());

                relay.abort();
            })
            .await
            .unwrap();
        });
    }
}
