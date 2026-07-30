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
}
