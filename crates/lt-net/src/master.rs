//! The public server list (`lt-master-proto`): a server's reports to the master server on a
//! thread of their own ([`Reporter`], native only), a game's look at the list
//! ([`fetch_list_into`]), and the way through the master server's relay ([`relay_url`]).

use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use lt_master_proto::API_SERVERS;
use lt_master_proto::{RELAY_PATH, ServerEntry};

pub use lt_master_proto::DEFAULT_MASTER;

#[cfg(not(target_arch = "wasm32"))]
mod http;
#[cfg(not(target_arch = "wasm32"))]
pub use http::{Reporter, fetch_list};

/// Where an answer to [`fetch_list_into`] goes.
pub type ListSlot = Arc<Mutex<Option<Result<Vec<ServerEntry>, String>>>>;

/// Fetches the list at `master` on a thread of its own, into `slot` when it is there (for a
/// game's menu: poll the slot each frame).
#[cfg(not(target_arch = "wasm32"))]
pub fn fetch_list_into(master: &str, slot: ListSlot) {
    let master = master.to_string();
    std::thread::spawn(move || {
        let list = fetch_list(&master).map_err(|e| format!("{e:#}"));
        if let Ok(mut s) = slot.lock() {
            *s = Some(list);
        }
    });
}

/// Fetches the list at `master` with the browser's `fetch`, into `slot` when it is there.
#[cfg(target_arch = "wasm32")]
pub fn fetch_list_into(master: &str, slot: ListSlot) {
    let url = format!("{}{API_SERVERS}", master.trim_end_matches('/'));
    wasm_bindgen_futures::spawn_local(async move {
        let list = fetch_json(&url).await;
        if let Ok(mut s) = slot.lock() {
            *s = Some(list);
        }
    });
}

#[cfg(target_arch = "wasm32")]
async fn fetch_json(url: &str) -> Result<Vec<ServerEntry>, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    let why = |e: wasm_bindgen::JsValue| {
        e.as_string()
            .or_else(|| {
                e.dyn_ref::<js_sys::Error>()
                    .map(|e| String::from(e.message()))
            })
            .unwrap_or_else(|| format!("{e:?}"))
    };
    let window = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("fetching {url}: {}", why(e)))?
        .unchecked_into();
    if !resp.ok() {
        return Err(format!("{url}: {} {}", resp.status(), resp.status_text()));
    }
    let text = JsFuture::from(resp.text().map_err(why)?)
        .await
        .map_err(why)?;
    serde_json::from_str(&text.as_string().unwrap_or_default())
        .map_err(|e| format!("reading {url}: {e}"))
}

/// The WebSocket URL of the relay at `master` ("https://host", or a "ws://"/"wss://" URL of a
/// relay of its own) to the server with this id ([`ServerEntry::id`]).
pub fn relay_url(master: &str, id: &str) -> String {
    let master = master.trim_end_matches('/');
    let ws = if master.starts_with("ws://") || master.starts_with("wss://") {
        master.to_string()
    } else if let Some(rest) = master.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = master.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("wss://{master}")
    };
    format!("{ws}{RELAY_PATH}{id}")
}

#[cfg(test)]
mod tests {
    use super::relay_url;

    #[test]
    fn relay_urls() {
        assert_eq!(
            relay_url("https://lt-master.example.com/", "abc"),
            "wss://lt-master.example.com/relay/abc"
        );
        assert_eq!(
            relay_url("http://127.0.0.1:27500", "abc"),
            "ws://127.0.0.1:27500/relay/abc"
        );
        assert_eq!(
            relay_url("ws://127.0.0.1:27501", "abc"),
            "ws://127.0.0.1:27501/relay/abc"
        );
        assert_eq!(
            relay_url("example.com", "abc"),
            "wss://example.com/relay/abc"
        );
    }
}
