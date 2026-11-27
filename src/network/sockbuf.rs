//! Tampons des prises UDP agrandis.
//!
//! Windows ne garde par défaut que quelques dizaines de Ko de datagrammes en attente sur une
//! prise UDP. Le serveur reçoit tous les joueurs sur une seule prise : un à-coup de quelques
//! millisecondes du fil de lecture pendant une rafale (explosions, chunks) suffit à remplir ce
//! tampon, et les datagrammes suivants sont jetés sans bruit — y compris les vérifications de
//! la liaison WebRTC, dont la perte fait couper des joueurs pour « liaison perdue ».

use std::net::UdpSocket;

/// Demande au système des tampons d'envoi et de réception de `bytes` octets (il peut en
/// accorder moins). Renvoie faux si le système a refusé.
pub fn enlarge(socket: &UdpSocket, bytes: usize) -> bool {
    let size = bytes.min(i32::MAX as usize) as i32;
    imp::set(socket, imp::SO_RCVBUF, size) & imp::set(socket, imp::SO_SNDBUF, size)
}

#[cfg(windows)]
mod imp {
    use std::net::UdpSocket;
    use std::os::windows::io::AsRawSocket;

    const SOL_SOCKET: i32 = 0xffff;
    pub const SO_SNDBUF: i32 = 0x1001;
    pub const SO_RCVBUF: i32 = 0x1002;

    #[link(name = "ws2_32")]
    extern "system" {
        fn setsockopt(s: usize, level: i32, name: i32, value: *const i32, len: i32) -> i32;
    }

    pub fn set(socket: &UdpSocket, name: i32, size: i32) -> bool {
        // SAFETY : la prise est ouverte tant que `socket` vit ; la valeur est un entier lu
        // pendant l'appel seulement.
        unsafe { setsockopt(socket.as_raw_socket() as usize, SOL_SOCKET, name, &size, std::mem::size_of::<i32>() as i32) == 0 }
    }
}

#[cfg(unix)]
mod imp {
    use std::net::UdpSocket;
    use std::os::unix::io::AsRawFd;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const SOL_SOCKET: i32 = 1;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub const SO_SNDBUF: i32 = 7;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub const SO_RCVBUF: i32 = 8;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const SOL_SOCKET: i32 = 0xffff;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    pub const SO_SNDBUF: i32 = 0x1001;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    pub const SO_RCVBUF: i32 = 0x1002;

    extern "C" {
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const i32, len: u32) -> i32;
    }

    pub fn set(socket: &UdpSocket, name: i32, size: i32) -> bool {
        // SAFETY : comme ci-dessus.
        unsafe { setsockopt(socket.as_raw_fd(), SOL_SOCKET, name, &size, std::mem::size_of::<i32>() as u32) == 0 }
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub const SO_SNDBUF: i32 = 0;
    pub const SO_RCVBUF: i32 = 0;
    pub fn set(_: &std::net::UdpSocket, _: i32, _: i32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn buffers_can_be_enlarged() {
        let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        assert!(super::enlarge(&s, 4 << 20));
    }
}
