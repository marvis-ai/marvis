//! `marvis://` deep-link routing.
//!
//! Dispatch is decoupled (same pattern as `hotkey.rs`): this module
//! reports [`Action`]s through a caller-supplied closure — `ask` gating
//! and `AppState` wiring live in `lib.rs`, which passes a `dispatch`
//! closure that maps `Ask` onto `ask::send` (gate `Main` only) and
//! `Focus` onto bar focus.
//!
//! Routes (deliberately minimal — no auth/Firebase callbacks):
//! - `marvis://ask?text=<percent-encoded>` → [`Action::Ask`]
//! - `marvis://ask` with no/empty `text` → [`Action::Focus`]
//! - any other `marvis://*` → [`Action::Focus`]
//! - non-`marvis` scheme / unparseable → [`Action::Ignore`]

use tauri::AppHandle;
use tauri_plugin_deep_link::DeepLinkExt;

/// A routed deep link's intent. Task 14 maps each variant onto the real
/// ask/focus calls inside the `dispatch` closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `marvis://ask?text=...` — ask the decoded text.
    Ask(String),
    /// Any other `marvis://*` — surface the bar.
    Focus,
    /// Not ours — leave it alone.
    Ignore,
}

/// Pure URL → [`Action`] routing, unit-tested without a runtime.
///
/// The `text` param is percent-decoded (`+` counts as space, per the
/// form-urlencoded convention every URL stack uses for query strings).
/// Scheme and host compare case-insensitively; `url::Url`-normalized
/// forms (`marvis://ask/?text=…`, trailing-slash host paths) route the
/// same as their spelled-out originals.
pub fn route(url: &str) -> Action {
    let Some((scheme, rest)) = url.split_once(':') else {
        return Action::Ignore;
    };
    if scheme.is_empty() || !scheme.eq_ignore_ascii_case("marvis") {
        return Action::Ignore;
    }
    // Drop the authority marker if present; `marvis:ask?…` routes too.
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    // Fragment never carries routing data — cut it first.
    let rest = rest.split('#').next().unwrap_or_default();
    // Host is the segment up to the first `/` or `?`. A real `url::Url`
    // serializes `marvis://ask?…` as `marvis://ask/?…`, so path after
    // the host is ignored for routing.
    let host_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (host, after_host) = rest.split_at(host_end);
    if !host.eq_ignore_ascii_case("ask") {
        return Action::Focus;
    }
    let query = after_host
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default();
    match query_param(query, "text") {
        Some(text) if !text.is_empty() => Action::Ask(text),
        _ => Action::Focus,
    }
}

/// First `key=value` pair in a `&`-separated query string whose key is
/// `key`, percent-decoded. Bare `?key` counts as an empty value.
fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            return Some(percent_decode(v));
        }
    }
    None
}

/// Minimal percent-decoder (`%XX` → byte, `+` → space). Invalid or
/// truncated escapes pass through literally — a mangled `text` is still
/// worth asking, and `url`/`urlencoding` aren't in our deps.
fn percent_decode(input: &str) -> String {
    fn hex_val(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                match bytes
                    .get(i + 1)
                    .and_then(|&h| hex_val(h))
                    .zip(bytes.get(i + 2).and_then(|&l| hex_val(l)))
                {
                    Some((h, l)) => {
                        out.push(h * 16 + l);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Listen for `marvis://` open-URL events and dispatch routed actions.
///
/// The plugin's `deep-link://new-url` event can carry multiple URLs —
/// each is routed and dispatched in order; `Ignore`s are skipped.
///
/// **Panics** if the deep-link plugin isn't registered (Task 14 adds it
/// to the `.plugin()` chain): `app.deep_link()` resolves managed state.
pub fn init(
    app: &AppHandle,
    dispatch: impl Fn(Action) + Send + Sync + 'static,
) -> anyhow::Result<()> {
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            match route(url.as_str()) {
                Action::Ignore => log::debug!("deeplink: ignoring {url}"),
                action => dispatch(action),
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_with_percent_encoded_text() {
        assert_eq!(
            route("marvis://ask?text=hello%20world"),
            Action::Ask("hello world".to_string())
        );
    }

    #[test]
    fn ask_empty_or_missing_text_focuses() {
        // Nothing to ask → just surface the bar.
        assert_eq!(route("marvis://ask"), Action::Focus);
        assert_eq!(route("marvis://ask?text="), Action::Focus);
        assert_eq!(route("marvis://ask?text"), Action::Focus);
        assert_eq!(route("marvis://ask?other=1"), Action::Focus);
    }

    #[test]
    fn other_marvis_paths_focus() {
        assert_eq!(route("marvis://focus"), Action::Focus);
        assert_eq!(route("marvis://other/path"), Action::Focus);
        assert_eq!(route("marvis://"), Action::Focus);
    }

    #[test]
    fn non_marvis_schemes_ignore() {
        assert_eq!(route("https://x"), Action::Ignore);
        assert_eq!(route("file:///etc/passwd"), Action::Ignore);
    }

    #[test]
    fn malformed_urls_ignore() {
        assert_eq!(route(""), Action::Ignore);
        assert_eq!(route("not a url"), Action::Ignore);
        assert_eq!(route("ask?text=hi"), Action::Ignore);
        assert_eq!(route("://ask?text=hi"), Action::Ignore);
    }

    #[test]
    fn scheme_and_host_are_case_insensitive() {
        // `MARVIS://ASK` — scheme (and host) casing is normalized by
        // real URL parsers; match that leniency.
        assert_eq!(route("MARVIS://ask?text=hi"), Action::Ask("hi".to_string()));
        assert_eq!(route("marvis://ASK?text=hi"), Action::Ask("hi".to_string()));
    }

    #[test]
    fn multiple_params_finds_text() {
        assert_eq!(
            route("marvis://ask?a=1&text=hi"),
            Action::Ask("hi".to_string())
        );
        assert_eq!(
            route("marvis://ask?text=hi&b=2"),
            Action::Ask("hi".to_string())
        );
        // First `text` wins; `Text` is a different (unknown) key.
        assert_eq!(
            route("marvis://ask?text=a&text=b"),
            Action::Ask("a".to_string())
        );
        assert_eq!(route("marvis://ask?Text=hi"), Action::Focus);
    }

    #[test]
    fn percent_decoding_edge_cases() {
        // Multi-byte UTF-8, `+` as space (form-urlencoded convention),
        // and literal passthrough of bad escapes.
        assert_eq!(
            route("marvis://ask?text=%F0%9F%98%80"),
            Action::Ask("😀".to_string())
        );
        assert_eq!(
            route("marvis://ask?text=a+b"),
            Action::Ask("a b".to_string())
        );
        assert_eq!(
            route("marvis://ask?text=100%zz%"),
            Action::Ask("100%zz%".to_string())
        );
    }

    #[test]
    fn url_crate_normalized_forms_route() {
        // At runtime the plugin hands us `url::Url`s; `Url::parse`
        // serializes `marvis://ask?text=hi` as `marvis://ask/?text=hi`
        // (empty path under an authority becomes `/`). Routing must
        // survive that normalization.
        assert_eq!(
            route("marvis://ask/?text=hi"),
            Action::Ask("hi".to_string())
        );
        assert_eq!(route("marvis://ask/"), Action::Focus);
    }

    #[test]
    fn fragment_is_ignored() {
        assert_eq!(
            route("marvis://ask?text=hi#frag"),
            Action::Ask("hi".to_string())
        );
    }
}
