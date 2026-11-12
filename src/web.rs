//! The browser build's bits of the browser (docs/web.md): the page's parameters and
//! localStorage.

/// The page's `?name=value` parameter.
pub fn param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search)
        .ok()?
        .get(name)
        .filter(|v| !v.is_empty())
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// What localStorage keeps under `key`.
pub fn load(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

/// Keeps `value` in localStorage under `key`.
pub fn store(key: &str, value: &str) -> Result<(), String> {
    storage()
        .ok_or("no localStorage")?
        .set_item(key, value)
        .map_err(|e| format!("{e:?}"))
}
