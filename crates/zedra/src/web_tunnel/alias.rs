//! Domain-alias adapter: an ephemeral in-app SOCKS5 proxy the webview is pointed
//! at via `proxyConfigurations`. Each host gets a per-host alias hostname
//! `<word>.zedra.test`; because it is non-loopback, WKWebView routes it through
//! the proxy (a real device bypasses the proxy for all loopback — see
//! `docs/WEB_TUNNEL_MODES.md`). The proxy decodes the label back to the host's
//! endpoint id and forwards over that host's `WebConnect`. Opt-in fallback used
//! only when exact-port can't bind.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Mutex, OnceLock};

use iroh::PublicKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OnceCell;

use super::bridge;

const ALIAS_SUFFIX: &str = ".zedra.test";

/// Short, DNS-safe words used as alias labels. Curated to be lowercase, exactly
/// four letters, unambiguous, and inoffensive; picked deterministically per host.
const WORDS: &[&str] = &[
    "puma", "fern", "reef", "dune", "opal", "jade", "lark", "moss", "pine", "sage", "teal", "wren",
    "kiwi", "lynx", "mint", "iris", "onyx", "ruby", "neon", "aqua", "flux", "echo", "halo", "peak",
    "vibe", "zest", "mist", "dawn", "dusk", "fawn", "hawk", "ibis", "newt", "swan", "seal", "orca",
    "wolf", "bear", "deer", "moth", "frog", "toad", "gull", "tern", "kelp", "palm", "reed", "vine",
    "star", "moon", "nova", "atom", "volt", "byte", "node", "mesh", "grid", "wave", "beam", "glow",
    "plum", "pear", "lime", "date", "corn", "oats", "rice", "bean", "herb", "leaf", "root", "seed",
    "bark", "jazz", "coal", "gold", "iron", "zinc", "clay", "sand", "snow", "rain", "wind", "foam",
    "tide", "cove", "isle", "silt", "brie", "kale", "taro", "yuca",
];

struct State {
    proxy_port: OnceCell<u16>,
    labels: Mutex<HashMap<String, PublicKey>>,
    by_endpoint: Mutex<HashMap<PublicKey, String>>,
}

fn state() -> &'static State {
    static STATE: OnceLock<State> = OnceLock::new();
    STATE.get_or_init(|| State {
        proxy_port: OnceCell::new(),
        labels: Mutex::new(HashMap::new()),
        by_endpoint: Mutex::new(HashMap::new()),
    })
}

/// The per-host alias hostname (`<word>.zedra.test`). The word is assigned once
/// per host and reused, so the origin stays stable across reopens; the label is
/// registered so the proxy can route its CONNECTs back to `endpoint_id`.
pub(super) fn alias_host(endpoint_id: &PublicKey) -> String {
    if let Some(label) = state().by_endpoint.lock().unwrap().get(endpoint_id) {
        return format!("{label}{ALIAS_SUFFIX}");
    }
    let mut labels = state().labels.lock().unwrap();
    let label = mint_label(endpoint_id, &labels);
    labels.insert(label.clone(), *endpoint_id);
    drop(labels);
    state()
        .by_endpoint
        .lock()
        .unwrap()
        .insert(*endpoint_id, label.clone());
    format!("{label}{ALIAS_SUFFIX}")
}

/// Pick a word for `endpoint_id`: start at an index seeded from its bytes (so the
/// choice is stable per host across restarts) and linear-probe past any word
/// already taken by another host. Falls back to a hex label if every word is in
/// use (only possible with more concurrent hosts than words).
fn mint_label(endpoint_id: &PublicKey, taken: &HashMap<String, PublicKey>) -> String {
    let bytes = endpoint_id.as_bytes();
    let seed = usize::from(bytes[0]) | (usize::from(bytes[1]) << 8);
    for offset in 0..WORDS.len() {
        let word = WORDS[(seed + offset) % WORDS.len()];
        if !taken.contains_key(word) {
            return word.to_string();
        }
    }
    bytes[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Bind the shared SOCKS proxy on first use; returns its ephemeral loopback port.
pub(super) async fn ensure_proxy() -> Result<u16, String> {
    state()
        .proxy_port
        .get_or_try_init(|| async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .map_err(|e| format!("failed to bind SOCKS proxy: {e}"))?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            spawn_accept_loop(listener);
            tracing::info!("[debug:web-tunnel] alias SOCKS proxy on 127.0.0.1:{port}");
            Ok::<_, String>(port)
        })
        .await
        .copied()
}

fn spawn_accept_loop(listener: TcpListener) {
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let _ = handle_socks(stream).await;
            });
        }
    });
}

fn reply(status: u8) -> [u8; 10] {
    [0x05, status, 0x00, 0x01, 0, 0, 0, 0, 0, 0]
}

async fn handle_socks(mut stream: TcpStream) -> Result<(), String> {
    let mut greeting = [0u8; 2];
    stream
        .read_exact(&mut greeting)
        .await
        .map_err(|e| e.to_string())?;
    if greeting[0] != 0x05 {
        return Err("not a SOCKS5 client".to_string());
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|e| e.to_string())?;
    stream
        .write_all(&[0x05, 0x00])
        .await
        .map_err(|e| e.to_string())?;

    let mut request = [0u8; 4];
    stream
        .read_exact(&mut request)
        .await
        .map_err(|e| e.to_string())?;
    if request[1] != 0x01 {
        stream.write_all(&reply(0x07)).await.ok();
        return Err("only SOCKS CONNECT is supported".to_string());
    }
    let host = match request[3] {
        0x01 => {
            let mut a = [0u8; 4];
            stream.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            Ipv4Addr::from(a).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|e| e.to_string())?;
            let mut domain = vec![0u8; len[0] as usize];
            stream
                .read_exact(&mut domain)
                .await
                .map_err(|e| e.to_string())?;
            String::from_utf8_lossy(&domain).into_owned()
        }
        0x04 => {
            let mut a = [0u8; 16];
            stream.read_exact(&mut a).await.map_err(|e| e.to_string())?;
            Ipv6Addr::from(a).to_string()
        }
        _ => {
            stream.write_all(&reply(0x08)).await.ok();
            return Err("unsupported SOCKS address type".to_string());
        }
    };
    let mut port_bytes = [0u8; 2];
    stream
        .read_exact(&mut port_bytes)
        .await
        .map_err(|e| e.to_string())?;
    let port = u16::from_be_bytes(port_bytes);

    match endpoint_for_host(&host) {
        Some(endpoint_id) => forward_via_session(stream, endpoint_id, port).await,
        None => forward_direct(stream, host, port).await,
    }
}

/// Resolve `<label>.zedra.test` back to the owning host endpoint id.
fn endpoint_for_host(host: &str) -> Option<PublicKey> {
    let label = host.strip_suffix(ALIAS_SUFFIX)?;
    state().labels.lock().unwrap().get(label).copied()
}

async fn forward_via_session(
    mut stream: TcpStream,
    endpoint_id: PublicKey,
    port: u16,
) -> Result<(), String> {
    let Some(session) = super::session_for(&endpoint_id) else {
        stream.write_all(&reply(0x05)).await.ok();
        return Err("no session for alias host".to_string());
    };
    let (tx, rx, initial) = match bridge::connect(&session, port).await {
        Ok(parts) => parts,
        Err(error) => {
            stream.write_all(&reply(0x05)).await.ok();
            return Err(error);
        }
    };
    stream
        .write_all(&reply(0x00))
        .await
        .map_err(|e| e.to_string())?;
    bridge::pump(stream, tx, rx, initial, |_| {}).await;
    Ok(())
}

/// Non-alias hosts (an external link followed in the webview) dial directly —
/// the session only ever forwards a host's loopback.
async fn forward_direct(mut stream: TcpStream, host: String, port: u16) -> Result<(), String> {
    let mut upstream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(upstream) => upstream,
        Err(error) => {
            stream.write_all(&reply(0x05)).await.ok();
            return Err(format!("direct dial {host}:{port} failed: {error}"));
        }
    };
    stream
        .write_all(&reply(0x00))
        .await
        .map_err(|e| e.to_string())?;
    tokio::io::copy_bidirectional(&mut stream, &mut upstream)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn words_are_dns_safe_and_unique() {
        let mut seen = HashSet::new();
        for word in WORDS {
            assert_eq!(word.len(), 4, "{word} is not 4 letters");
            assert!(
                word.bytes().all(|b| b.is_ascii_lowercase()),
                "{word} is not lowercase a-z"
            );
            assert!(seen.insert(*word), "{word} is duplicated");
        }
    }

    #[test]
    fn mint_is_deterministic_for_a_host() {
        let key = iroh::SecretKey::from_bytes(&[7u8; 32]).public();
        let taken = HashMap::new();
        // Same host + same taken set always yields the same word (stable label).
        assert_eq!(mint_label(&key, &taken), mint_label(&key, &taken));
    }

    #[test]
    fn mint_probes_past_taken_words() {
        let key = iroh::SecretKey::from_bytes(&[7u8; 32]).public();
        let other = iroh::SecretKey::from_bytes(&[9u8; 32]).public();
        let mut taken = HashMap::new();
        let first = mint_label(&key, &taken);
        assert!(WORDS.contains(&first.as_str()));
        // With that word taken by someone else, the same host picks a different one.
        taken.insert(first.clone(), other);
        let second = mint_label(&key, &taken);
        assert_ne!(first, second);
        assert!(WORDS.contains(&second.as_str()));
    }

    #[test]
    fn mint_falls_back_to_hex_when_every_word_is_taken() {
        let key = iroh::SecretKey::from_bytes(&[7u8; 32]).public();
        let other = iroh::SecretKey::from_bytes(&[9u8; 32]).public();
        let taken: HashMap<String, PublicKey> =
            WORDS.iter().map(|w| (w.to_string(), other)).collect();
        let label = mint_label(&key, &taken);
        assert_eq!(label.len(), 16);
        assert!(label.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!WORDS.contains(&label.as_str()));
    }
}
