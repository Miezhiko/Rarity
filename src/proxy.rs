// Helpers for running the bot behind an outbound proxy (e.g. a VPN egress
// node). Everything here is opt-in via environment variables so a deployment
// that sets none of them behaves exactly as before.

use reqwest::{ Client, NoProxy, Proxy };

fn env_first(names: &[&str]) -> Option<String> {
  names.iter().find_map(|name| std::env::var(name).ok())
}

/// Reads the conventional proxy environment variables, preferring the most
/// specific one set.
pub fn env_proxy_url() -> Option<String> {
  env_first(&["ALL_PROXY", "all_proxy", "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"])
}

/// Reads the conventional `NO_PROXY` environment variable, if any.
pub fn env_no_proxy() -> String {
  env_first(&["NO_PROXY", "no_proxy"]).unwrap_or_default()
}

/// Builds the shared HTTP client used for Ollama and RSS requests.
///
/// When `proxy_url` is set, all requests are routed through it except for
/// localhost, so a proxy configured for reaching the outside world never
/// breaks the local Ollama API even if `existing_no_proxy` doesn't already
/// exclude it.
pub fn build_request_client(proxy_url: Option<&str>, existing_no_proxy: &str) -> reqwest::Result<Client> {
  let mut builder = Client::builder().pool_max_idle_per_host(0);

  if let Some(url) = proxy_url {
    let no_proxy = format!("{existing_no_proxy},localhost,127.0.0.1,::1");
    builder = builder.proxy(Proxy::all(url)?.no_proxy(NoProxy::from_string(&no_proxy)));
  }

  builder.build()
}

/// Resolves the Discord REST API proxy from already-read environment values.
///
/// This targets `twilight-http`'s own proxy support, i.e. a self-hosted
/// mirror such as <https://github.com/twilight-rs/http-proxy> that the bot
/// can reach from inside a private/VPN network without needing direct
/// internet access to discord.com. `use_http` accepts "1"/"true".
fn discord_http_proxy_from(url: Option<String>, use_http_raw: Option<String>) -> Option<(String, bool)> {
  let url = url?;
  let use_http = use_http_raw
    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    .unwrap_or(false);
  Some((url, use_http))
}

/// Reads the optional `DISCORD_HTTP_PROXY_URL`/`DISCORD_HTTP_PROXY_USE_HTTP`
/// environment variables. Absent `DISCORD_HTTP_PROXY_URL`, behavior is
/// unchanged: the REST client talks to discord.com directly.
pub fn env_discord_http_proxy() -> Option<(String, bool)> {
  discord_http_proxy_from(
    std::env::var("DISCORD_HTTP_PROXY_URL").ok(),
    std::env::var("DISCORD_HTTP_PROXY_USE_HTTP").ok()
  )
}

/// Reads the optional `DISCORD_GATEWAY_PROXY_URL` environment variable, used
/// to point the gateway websocket connection at a self-hosted mirror (e.g.
/// twilight-rs/gateway-proxy) instead of connecting to Discord directly.
/// Absent this variable, behavior is unchanged.
pub fn env_discord_gateway_proxy_url() -> Option<String> {
  std::env::var("DISCORD_GATEWAY_PROXY_URL").ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn no_proxy_configured_builds_default_client() {
    assert!(build_request_client(None, "").is_ok());
  }

  #[test]
  fn proxy_configured_still_builds_a_client() {
    assert!(build_request_client(Some("http://127.0.0.1:9"), "example.internal").is_ok());
  }

  #[test]
  fn invalid_proxy_url_is_rejected() {
    assert!(build_request_client(Some("not a url"), "").is_err());
  }

  #[test]
  fn no_discord_http_proxy_url_means_no_proxy() {
    assert_eq!(discord_http_proxy_from(None, Some("true".into())), None);
  }

  #[test]
  fn discord_http_proxy_defaults_to_https() {
    assert_eq!(
      discord_http_proxy_from(Some("proxy.internal".into()), None),
      Some(("proxy.internal".into(), false))
    );
  }

  #[test]
  fn discord_http_proxy_use_http_is_parsed() {
    assert_eq!(
      discord_http_proxy_from(Some("proxy.internal".into()), Some("true".into())),
      Some(("proxy.internal".into(), true))
    );
  }
}
