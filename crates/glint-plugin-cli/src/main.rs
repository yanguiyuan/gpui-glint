use clap::{Parser, Subcommand};
use glint_plugin_registry::{Client, HostTarget, Package, PluginStore, Publish, Result, Source};
use std::path::PathBuf;
#[derive(Parser)]
#[command(version, about = "Publish and manage plugins on any Glint registry")]
struct Arguments {
    #[arg(long, env = "GLINT_PLUGIN_HOME", global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    Source {
        #[command(subcommand)]
        command: Sources,
    },
    List {
        #[arg(long)]
        source: String,
    },
    Publish {
        directory: PathBuf,
        #[arg(long)]
        source: String,
        #[arg(long = "host", required = true)]
        hosts: Vec<String>,
        #[arg(long, default_value = "")]
        changelog: String,
    },
    Yank {
        id: String,
        version: String,
        #[arg(long)]
        source: String,
    },
    Restore {
        id: String,
        version: String,
        #[arg(long)]
        source: String,
    },
    Install {
        id: String,
        version: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        host: String,
        #[arg(long)]
        host_version: String,
    },
    Installed,
    Uninstall {
        id: String,
        version: String,
        #[arg(long)]
        source: String,
    },
}
#[derive(Subcommand)]
enum Sources {
    Inspect {
        url: String,
        #[arg(long)]
        allow_http: bool,
    },
    Add {
        name: String,
        url: String,
        #[arg(long)]
        public_key: String,
        #[arg(long)]
        allow_http: bool,
    },
    List,
    Remove {
        name: String,
    },
}
fn home() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    Ok(base
        .ok_or("Configure --home or GLINT_PLUGIN_HOME")?
        .join("glint-pluginctl"))
}
fn output(value: impl serde::Serialize) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|_| "Cannot encode output")?
    );
    Ok(())
}
fn run(args: Arguments) -> Result<()> {
    let store = PluginStore::new(args.home.map(Ok).unwrap_or_else(home)?)?;
    let client = Client::new()?;
    let token = || {
        std::env::var("GLINT_PLUGIN_TOKEN")
            .map_err(|_| "Set GLINT_PLUGIN_TOKEN for management operations".to_owned())
    };
    match args.command {
        Commands::Source { command } => match command {
            Sources::Inspect { url, allow_http } => output(client.inspect(&url, allow_http)?),
            Sources::Add {
                name,
                url,
                public_key,
                allow_http,
            } => {
                let source = Source {
                    name,
                    url,
                    public_key,
                    allow_http,
                };
                store.add_source(source.clone())?;
                output(source)
            }
            Sources::List => output(store.sources()?),
            Sources::Remove { name } => store.remove_source(&name),
        },
        Commands::List { source } => output(store.refresh(&client, &store.source(&source)?)?),
        Commands::Publish {
            directory,
            source,
            hosts,
            changelog,
        } => {
            let plugin = glint_plugin::Plugin::load(&directory).map_err(|e| e.to_string())?;
            let bytes = Package::encode(plugin)?;
            let hosts = hosts
                .into_iter()
                .map(|h| {
                    let (id, min_version) = h
                        .split_once('@')
                        .ok_or("--host must be application@minimum-version")?;
                    Ok(HostTarget {
                        id: id.into(),
                        min_version: min_version.into(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            output(client.publish(
                &store.source(&source)?,
                &token()?,
                &Publish::new(&bytes, hosts, changelog),
            )?)
        }
        Commands::Yank {
            id,
            version,
            source,
        } => output(client.set_yanked(&store.source(&source)?, &token()?, &id, &version, true)?),
        Commands::Restore {
            id,
            version,
            source,
        } => output(client.set_yanked(&store.source(&source)?, &token()?, &id, &version, false)?),
        Commands::Install {
            id,
            version,
            source,
            host,
            host_version,
        } => output(store.install(
            &client,
            &store.source(&source)?,
            &id,
            &version,
            &host,
            &host_version,
        )?),
        Commands::Installed => output(store.installed()?),
        Commands::Uninstall {
            id,
            version,
            source,
        } => store.uninstall(&source, &id, &version),
    }
}
fn main() {
    if let Err(error) = run(Arguments::parse()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
