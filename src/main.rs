use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use basalt::config::Config;
use basalt::server::Server;
use basalt::{error, info};

static STOP: AtomicBool = AtomicBool::new(false);

/// Arrêt propre sur Ctrl+C / SIGTERM (Unix). Sous Windows, utilisez la commande `stop`.
#[cfg(unix)]
fn install_signals() {
    extern "C" fn on_signal(_: i32) {
        STOP.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    extern "C" {
        fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
    }
    unsafe {
        signal(2, on_signal);
        signal(15, on_signal);
    }
}

#[cfg(not(unix))]
fn install_signals() {}

fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("basalt {}", basalt::VERSION);
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|a| a == "capture").unwrap_or(false) {
        capture(&args[2..]);
        return;
    }
    basalt::console::init();
    basalt::log::init_file();
    info!("Basalt {} — serveur Minecraft Bedrock", basalt::VERSION);
    let cfg = Config::load(Path::new("server.properties"));
    info!(
        "Versions acceptées : Bedrock {} à {} (protocoles {} à {})",
        basalt::versions::name(cfg.min_protocol),
        basalt::versions::name(cfg.max_protocol),
        cfg.min_protocol,
        cfg.max_protocol
    );

    let mut server = match Server::new(cfg.clone()) {
        Ok(s) => s,
        Err(e) => {
            error!("Impossible d'ouvrir {}:{} en UDP : {}", cfg.ip, cfg.port, e);
            basalt::console::restore();
            std::process::exit(1);
        }
    };
    install_signals();

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || basalt::console::read_commands(tx));

    server.run(&STOP, &rx);
    basalt::console::restore();
}

/// `basalt capture [adresse] [port]` : enregistre les données du jeu auprès d'un serveur officiel.
fn capture(args: &[String]) {
    let host = args.first().map(|s| s.as_str()).unwrap_or("127.0.0.1");
    let port: u16 = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(19132);
    basalt::log::init_capture_file();
    info!("Capture des données du jeu auprès du serveur officiel {}:{} (outil version 6, RakNet)", host, port);
    match basalt::tools::capture_rak::run(host, port, Path::new("data"), Path::new("logs/client-data.b64")) {
        Ok(summary) => info!("{}", summary),
        Err(e) => error!("Capture impossible : {}", e),
    }
}
