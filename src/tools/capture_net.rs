//! Transport de l'outil de capture : Basalt joue ici le rôle du jeu et se connecte au
//! serveur officiel par NetherNet (signalisation HTTP, puis liaison WebRTC).

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use str0m::change::SdpAnswer;
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

use crate::framing::{self, Reassembler};
use crate::identity::ServerIdentity;
use crate::signaling::{sdp_fingerprints, sdp_insert_identity, sdp_without_identity};
use crate::tools::capture::{build_login, Capture, Stage};

/// Requête HTTP simple ; renvoie (code, corps).
fn http(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| format!("connexion TCP à {} : {}", addr, e))?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    let head = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        method,
        path,
        addr,
        body.len()
    );
    stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body)).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.len() > 1_000_000 {
                    return Err("réponse HTTP trop longue".into());
                }
                // Réponse complète dès que le corps annoncé est arrivé
                if let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..split]).to_lowercase();
                    let wanted = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok());
                    if let Some(len) = wanted {
                        if raw.len() >= split + 4 + len {
                            break;
                        }
                    }
                    // Réponse envoyée par morceaux : elle se termine par un morceau vide
                    if head.contains("chunked") && raw.ends_with(b"0\r\n\r\n") {
                        break;
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("réponse HTTP incomplète")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("réponse HTTP illisible")?;
    let body = &raw[split + 4..];
    if head.to_lowercase().contains("chunked") {
        return Ok((status, crate::tools::capture::unchunk(body)));
    }
    Ok((status, body.to_vec()))
}

/// Adresse locale utilisée pour joindre le serveur officiel.
fn local_ip_for(remote: SocketAddr) -> Option<IpAddr> {
    let probe = UdpSocket::bind("0.0.0.0:0").ok()?;
    probe.connect(remote).ok()?;
    probe.local_addr().ok().map(|a| a.ip())
}

/// Cherche un nombre après `"clé":` dans un petit document JSON.
fn json_number(text: &str, key: &str) -> Option<i64> {
    let at = text.find(&format!("\"{}\"", key))?;
    let rest = &text[at + key.len() + 2..];
    let digits: String = rest.chars().skip_while(|c| !c.is_ascii_digit() && *c != '-').take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

fn write_message(rtc: &mut Rtc, channel: ChannelId, message: &[u8]) -> Result<(), String> {
    for frame in framing::split(message).map_err(|e| e.to_string())? {
        let mut ch = rtc.channel(channel).ok_or("canal fermé")?;
        let outcome = format!("{:?}", ch.write(true, &frame));
        if !outcome.starts_with("Ok") {
            return Err(format!("écriture refusée : {}", outcome));
        }
    }
    Ok(())
}

/// Lance la capture. `data_dir` reçoit les paquets, `payload` est le modèle de données client.
pub fn run(host: &str, port: u16, data_dir: &Path, payload: &Path) -> Result<String, String> {
    let server: SocketAddr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("adresse {} : {}", host, e))?
        .next()
        .ok_or("adresse introuvable")?;
    let client_payload = std::fs::read_to_string(payload).map_err(|_| {
        format!(
            "modèle introuvable ({}). Connectez-vous une fois à Basalt avec le jeu, debug-packets=true, puis relancez la capture.",
            payload.display()
        )
    })?;
    let identity = ServerIdentity::load_or_generate(Path::new("capture-identity.key"), "basalt.local", true)?;
    let now_unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    // 1. Le serveur officiel annonce sa version
    let (status, body) = http(server, "GET", "/v1/join?version=1.26.65&protocol=2225&platform=8&id=7205759403792793", b"")?;
    let text = String::from_utf8_lossy(&body).to_string();
    if status != 200 {
        return Err(format!("le serveur officiel répond {} à la demande d'informations : {}", status, crate::log::clean(&text, 200)));
    }
    let protocol = json_number(&text, "protocol").unwrap_or(2225) as i32;
    info!("Serveur officiel trouvé : protocole {} — {}", protocol, crate::log::clean(&text, 200));

    // 2. Offre WebRTC
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let local_ip = local_ip_for(SocketAddr::new(server.ip(), 19132)).ok_or("aucune adresse locale utilisable")?;
    let local = SocketAddr::new(local_ip, socket.local_addr().map_err(|e| e.to_string())?.port());
    let start = Instant::now();
    let mut rtc = Rtc::builder().build(start);
    let candidate = Candidate::host(local, "udp").map_err(|e| format!("candidat : {:?}", e))?;
    let _ = rtc.add_local_candidate(candidate);
    let mut api = rtc.sdp_api();
    let reliable = api.add_channel("ReliableDataChannel".to_string());
    let _unreliable = api.add_channel("UnreliableDataChannel".to_string());
    let (offer, pending) = api.apply().ok_or("offre WebRTC vide")?;
    let offer_sdp = offer.to_sdp_string();
    let signed = sdp_insert_identity(&offer_sdp, &identity.assertion(&sdp_fingerprints(&offer_sdp), now_unix));

    // 3. Signalisation. Le serveur officiel peut refuser une offre selon l'identité jointe :
    //    on essaie d'abord avec l'identité signée par Basalt, puis sans identité.
    let _ = std::fs::create_dir_all("logs");
    let _ = std::fs::write("logs/capture-offer.sdp", &offer_sdp);
    let attempts: [(&str, String); 2] = [("avec identité auto-signée", signed), ("sans identité", offer_sdp.clone())];
    let mut sdp_text = None;
    let mut refusals = Vec::new();
    for (label, body_out) in attempts.iter() {
        let (status, body) = http(server, "POST", "/v1/join/7205759403792793", body_out.as_bytes())?;
        let answer_text = String::from_utf8_lossy(&body).to_string();
        let _ = std::fs::write("logs/capture-answer.txt", &body);
        match answer_text.find("v=0") {
            Some(at) if status == 200 || status == 201 => {
                info!("Offre acceptée par le serveur officiel ({})", label);
                sdp_text = Some(answer_text[at..].to_string());
                break;
            }
            _ => {
                let shown = crate::log::clean(&answer_text, 120);
                info!("Offre refusée ({}) : HTTP {}, réponse « {} »", label, status, shown);
                refusals.push(format!("{} : HTTP {} « {} »", label, status, shown));
            }
        }
    }
    let sdp_text = sdp_text.ok_or_else(|| format!("le serveur officiel refuse l'offre de connexion — {}", refusals.join(" ; ")))?;
    let answer = SdpAnswer::from_sdp_string(&sdp_without_identity(&sdp_text))
        .map_err(|e| format!("réponse SDP illisible : {:?} — début : {}", e, crate::log::clean(&sdp_text, 200)))?;
    rtc.sdp_api().accept_answer(pending, answer).map_err(|e| format!("réponse SDP refusée : {:?}", e))?;

    // 4. Liaison et dialogue
    let login = build_login(protocol, "BasaltCapture", identity.spki_b64(), client_payload.trim(), &|m| identity.sign(m), now_unix);
    let mut capture = Capture::new(protocol, login);
    let mut inbound = Reassembler::new(64 * 1024 * 1024);
    let mut opened = false;
    let mut recording_since: Option<Instant> = None;
    let deadline = start + Duration::from_secs(40);
    let mut buf = vec![0u8; 2048];
    'main: loop {
        let now = Instant::now();
        if now > deadline {
            break;
        }
        if capture.stage == Stage::Failed || capture.complete {
            break;
        }
        if let Some(since) = recording_since {
            if now.duration_since(since) > Duration::from_secs(12) {
                break; // tout ce qui précède l'apparition est arrivé depuis longtemps
            }
        }
        // Tout ce que la liaison veut faire maintenant
        let wait_until = loop {
            match rtc.poll_output().map_err(|e| format!("liaison WebRTC : {:?}", e))? {
                Output::Timeout(at) => break at,
                Output::Transmit(t) => {
                    let _ = socket.send_to(&t.contents, t.destination);
                }
                Output::Event(event) => {
                    let mut replies: Vec<Vec<u8>> = Vec::new();
                    match event {
                        Event::ChannelOpen(id, label) => {
                            if id == reliable || label == "ReliableDataChannel" {
                                opened = true;
                                info!("Liaison ouverte avec le serveur officiel");
                                replies.push(capture.hello());
                            }
                        }
                        Event::ChannelData(data) => {
                            if data.id == reliable {
                                match inbound.push(&data.data) {
                                    Ok(Some(message)) => replies = capture.on_message(&message),
                                    Ok(None) => {}
                                    Err(why) => return Err(format!("message du serveur officiel illisible : {}", why)),
                                }
                                if capture.stage == Stage::Recording && recording_since.is_none() {
                                    recording_since = Some(Instant::now());
                                    info!("Login accepté par le serveur officiel : enregistrement en cours");
                                }
                            }
                        }
                        Event::ChannelClose(id) => {
                            if id == reliable {
                                break 'main;
                            }
                        }
                        _ => {}
                    }
                    for reply in replies {
                        write_message(&mut rtc, reliable, &reply)?;
                    }
                }
                #[allow(unreachable_patterns)]
                _ => {}
            }
        };
        if !rtc.is_alive() {
            break;
        }
        let wait = wait_until.saturating_duration_since(Instant::now()).min(Duration::from_millis(50)).max(Duration::from_millis(1));
        let _ = socket.set_read_timeout(Some(wait));
        match socket.recv_from(&mut buf) {
            Ok((n, source)) => {
                if let Ok(contents) = (&buf[..n]).try_into() {
                    let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: local, contents });
                    if rtc.accepts(&input) {
                        rtc.handle_input(input).map_err(|e| format!("liaison WebRTC : {:?}", e))?;
                    }
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                rtc.handle_input(Input::Timeout(Instant::now())).map_err(|e| format!("liaison WebRTC : {:?}", e))?;
            }
            Err(_) => {}
        }
    }
    rtc.disconnect();

    if let Some(why) = &capture.refusal {
        return Err(why.clone());
    }
    if !opened {
        return Err("la liaison WebRTC avec le serveur officiel ne s'est pas ouverte (pare-feu, ou serveur réglé sur transport=raknet ?)".into());
    }
    let found = capture.found();
    if found.is_empty() {
        return Err(format!(
            "aucun paquet utile reçu ({} paquets au total, étape {:?}). Vérifiez online-mode=false dans le server.properties du serveur officiel.",
            capture.records.len(),
            capture.stage
        ));
    }
    let dir = capture.save(data_dir).map_err(|e| format!("écriture des fichiers : {}", e))?;
    let list: Vec<String> = found.iter().map(|(name, size)| format!("{} ({} octets)", name, size)).collect();
    Ok(format!("Capture réussie pour le protocole {} : {} — rangés dans {}", protocol, list.join(", "), dir.display()))
}
