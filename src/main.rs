use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

use basalt::config::Config;
use basalt::server::Server;
use basalt::{error, info, warn};

static STOP: AtomicBool = AtomicBool::new(false);

/// Arrêt propre sur Ctrl+C / SIGTERM (Unix). Sous Windows, utilisez la commande `stop`.
#[cfg(unix)]
fn install_signals() {
    /// Ctrl+C ou arrêt du système : le serveur s'arrête proprement au tick suivant.
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

/// Autres systèmes : pas de gestion des signaux.
#[cfg(not(unix))]
fn install_signals() {}

/// Lit server.properties, démarre le serveur et la console, puis tourne jusqu'à l'arrêt.
fn main() {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("basalt {} {}", basalt::VERSION, option_env!("BASALT_BUILD").unwrap_or(""));
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|a| a == "capture").unwrap_or(false) {
        capture(&args[2..]);
        basalt::log::flush();
        return;
    }
    basalt::console::init();
    // La flèche du haut retrouve aussi les commandes des sessions précédentes
    basalt::console::load_history(Path::new("logs/console-history.txt"));
    basalt::log::init_file();
    // La compilation GitHub inscrit ici le commit et la date : on sait toujours quelle
    // version tourne en lisant la première ligne du journal
    match option_env!("BASALT_BUILD") {
        Some(build) if !build.is_empty() => info!("Basalt {} ({}) — serveur Minecraft Bedrock", basalt::VERSION, build),
        _ => info!("Basalt {} — serveur Minecraft Bedrock", basalt::VERSION),
    }
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
            basalt::log::flush();
            basalt::console::restore();
            std::process::exit(1);
        }
    };
    install_signals();

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || basalt::console::read_commands(tx));

    server.run(&STOP, &rx);
    // Les écritures en arrière-plan (disque, journal) sont terminées avant de quitter
    basalt::background::DISK.flush(std::time::Duration::from_secs(60));
    basalt::log::flush();
    basalt::console::restore();
}

/// `basalt capture [adresse] [port]` : enregistre les données du jeu auprès d'un serveur officiel.
fn capture(args: &[String]) {
    let host = args.first().map(|s| s.as_str()).unwrap_or("127.0.0.1");
    let port: u16 = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(19132);
    basalt::log::init_capture_file();
    info!("Capture des données du jeu auprès du serveur officiel {}:{} (outil version 14, RakNet, avec témoin, objet au sol et téléportation)", host, port);
    // Le témoin repart avant le joueur de la capture (20 s contre 35 s en jeu) : celui-ci reçoit
    // alors aussi le paquet qui retire un joueur de la liste.
    // Un second faux joueur, le « témoin », entre d'abord et reste en jeu : le serveur officiel
    // enverra alors au joueur de la capture le paquet qui fait apparaître un autre joueur.
    let witness_host = host.to_string();
    let witness = std::thread::spawn(move || {
        basalt::tools::capture_rak::run_as(&witness_host, port, None, Path::new("logs/client-data.b64"), "BasaltTemoin", std::time::Duration::from_secs(20), ("", ""))
    });
    std::thread::sleep(std::time::Duration::from_secs(8));
    info!("==================================================================================");
    info!("PENDANT LA CAPTURE (dans la minute qui suit), tapez dans la console du serveur officiel :");
    info!("    setblock 0 -60 0 dirt");
    info!("    setblock 0 -60 0 air destroy");
    info!("    tp @a 5 -59 5");
    info!("    summon cow 3 -59 3");
    info!("La deuxième fait tomber un objet au sol, la troisième téléporte le joueur de capture,");
    info!("la quatrième fait apparaître une vache :");
    info!("leurs formats seront relevés.");
    info!("==================================================================================");
    // Le serveur officiel refuse un second joueur qu'il prend pour le même que le premier
    // (« raison 44 »). Cause la plus probable : l'identifiant d'installation, vide dans le
    // modèle, donc identique pour les deux. D'autres identités sont essayées ensuite.
    let attempts: [(&str, &str, &str); 3] = [
        ("identifiant d'installation propre", "installation", ""),
        ("modèle intact", "", ""),
        ("identifiants de compte distincts", "B45A17C0DE000002", "2535400000000002"),
    ];
    let mut result = Err("aucun essai".to_string());
    for (label, mid, xid) in attempts.iter() {
        info!("Joueur de capture : essai avec {}", label);
        result = basalt::tools::capture_rak::run_as(host, port, Some(Path::new("data")), Path::new("logs/client-data.b64"), "BasaltCapture", std::time::Duration::from_secs(60), (mid, xid));
        match &result {
            // Tout refus au moment du login : on passe à l'identité suivante
            Err(e) if e.contains("Login") => warn!("Refusé ({}) : essai suivant", e),
            _ => break,
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    match result {
        Ok(summary) => {
            info!("{}", summary);
            if !summary.contains(basalt::tools::capture::REMOVAL_FILE) {
                warn!("Le retrait d'un joueur de la liste n'a pas été reçu : le témoin est sans doute parti trop tôt ou trop tard.");
            }
            if !summary.contains("add_player") {
                warn!("Le paquet d'apparition d'un autre joueur n'a pas été reçu : le témoin n'était sans doute pas en jeu.");
            }
        }
        Err(e) => error!("Capture impossible : {}", e),
    }
    match witness.join() {
        Ok(Ok(summary)) => info!("{}", summary),
        Ok(Err(e)) => warn!("Témoin : {}", e),
        Err(_) => warn!("Témoin : arrêt inattendu"),
    }
}
