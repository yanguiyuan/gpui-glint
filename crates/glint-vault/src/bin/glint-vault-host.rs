fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let executable = std::env::current_exe().ok();
    let installed_data = executable
        .as_ref()
        .and_then(|path| path.parent())
        .filter(|parent| parent.file_name().is_some_and(|name| name == "native-host"))
        .and_then(|parent| parent.parent());
    let database = installed_data
        .map(|path| path.join("vault.sqlite3"))
        .unwrap_or_else(glint_vault::default_path);
    let result = if args.first().is_some_and(|value| value == "--install") {
        if args.len() != 3 {
            Err("用法：glint-vault-host --install chrome|edge|egolite EXTENSION_ID".into())
        } else {
            glint_vault::browser::install_host(&database, &args[1], &args[2])
                .map(|path| println!("已注册：{}", path.display()))
        }
    } else {
        glint_vault::browser::native_host(
            &database,
            args.first().map(String::as_str).unwrap_or(""),
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        )
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
