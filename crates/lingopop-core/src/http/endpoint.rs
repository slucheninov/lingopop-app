//! Endpoint URL builder (port of `ProviderHTTP.endpoint` from the macOS app).

use url::Url;

use crate::errors::TranslationError;

fn https_only() -> TranslationError {
    TranslationError::InvalidConfiguration("baseURL must use HTTPS scheme".to_string())
}

/// The text before the first `:` (the whole input when there is none).
fn scheme_of(base: &str) -> &str {
    base.split(':').next().unwrap_or(base)
}

/// Builds `base_url + path (+ query)`. An empty or whitespace-only `base_url` falls back to
/// `default_base_url`. Only `https` is accepted; user info, query and fragment of the base
/// are dropped, trailing slashes of its path are stripped, and `query` pairs are
/// form-encoded in order. The scheme is checked before the host, like the Swift code:
/// `http://` is a configuration error, `https://` an invalid URL.
pub fn endpoint(
    base_url: &str,
    default_base_url: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Url, TranslationError> {
    let trimmed = base_url.trim();
    let base = if trimmed.is_empty() {
        default_base_url
    } else {
        trimmed
    };
    let mut url = match Url::parse(base) {
        Ok(u) => u,
        Err(url::ParseError::EmptyHost) if scheme_of(base).eq_ignore_ascii_case("https") => {
            return Err(TranslationError::InvalidUrl)
        }
        Err(_) => return Err(https_only()),
    };
    if url.scheme() != "https" {
        return Err(https_only());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(TranslationError::InvalidUrl);
    }
    let base_path = url.path().trim_end_matches('/').to_owned();
    let extra = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.set_path(&format!("{base_path}{extra}"));
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = "https://api.example.com";

    fn build(base: &str) -> Result<String, TranslationError> {
        endpoint(base, DEFAULT, "/v1/x", &[]).map(|u| u.to_string())
    }

    fn https_only_error() -> TranslationError {
        TranslationError::InvalidConfiguration("baseURL must use HTTPS scheme".into())
    }

    #[test]
    fn endpoint_uses_default_base_when_empty() {
        assert_eq!(build("").unwrap(), "https://api.example.com/v1/x");
    }

    #[test]
    fn endpoint_whitespace_only_base_uses_default() {
        assert_eq!(build("  \n ").unwrap(), "https://api.example.com/v1/x");
    }

    #[test]
    fn endpoint_trims_whitespace_around_base() {
        assert_eq!(
            build("  https://llm.example.com  ").unwrap(),
            "https://llm.example.com/v1/x"
        );
    }

    #[test]
    fn endpoint_rejects_http_scheme() {
        assert_eq!(build("http://llm.example.com"), Err(https_only_error()));
    }

    #[test]
    fn endpoint_scheme_check_is_case_insensitive() {
        assert_eq!(
            build("HTTPS://Example.COM").unwrap(),
            "https://example.com/v1/x"
        );
    }

    #[test]
    fn endpoint_unparseable_base_is_invalid_configuration() {
        assert_eq!(build("not a url"), Err(https_only_error()));
        assert_eq!(build("//h"), Err(https_only_error()));
    }

    #[test]
    fn endpoint_other_parse_errors_are_invalid_configuration() {
        // An out-of-range port is a parse error that is neither "empty host" nor a scheme
        // problem; it maps to the same configuration error as any unparseable base.
        assert_eq!(
            build("https://api.example.com:99999"),
            Err(https_only_error())
        );
        assert_eq!(
            build("https://api.example.com:abc"),
            Err(https_only_error())
        );
        assert_eq!(build("https://exa mple.com"), Err(https_only_error()));
    }

    #[test]
    fn endpoint_empty_host_is_invalid_url() {
        assert_eq!(build("https://"), Err(TranslationError::InvalidUrl));
        assert_eq!(build("https://:443"), Err(TranslationError::InvalidUrl));
    }

    #[test]
    fn endpoint_http_with_empty_host_is_invalid_configuration() {
        assert_eq!(build("http://"), Err(https_only_error()));
    }

    #[test]
    fn endpoint_drops_userinfo_query_and_fragment() {
        assert_eq!(
            build("https://u:pw@h.example.com/p/?a=1#f").unwrap(),
            "https://h.example.com/p/v1/x"
        );
    }

    #[test]
    fn endpoint_keeps_explicit_port() {
        assert_eq!(
            build("https://h.example.com:8443").unwrap(),
            "https://h.example.com:8443/v1/x"
        );
        assert_eq!(
            build("https://[::1]:8443").unwrap(),
            "https://[::1]:8443/v1/x"
        );
    }

    #[test]
    fn endpoint_normalizes_default_port() {
        // The url crate drops :443; Swift would keep it, equivalent in practice.
        assert_eq!(
            build("https://h.example.com:443").unwrap(),
            "https://h.example.com/v1/x"
        );
    }

    #[test]
    fn endpoint_strips_all_trailing_slashes_of_base_path() {
        assert_eq!(
            build("https://p.example.com/litellm///").unwrap(),
            "https://p.example.com/litellm/v1/x"
        );
    }

    #[test]
    fn endpoint_keeps_base_path() {
        assert_eq!(
            build("https://p.example.com/a/b").unwrap(),
            "https://p.example.com/a/b/v1/x"
        );
    }

    #[test]
    fn endpoint_prepends_slash_to_path() {
        let a = endpoint("", DEFAULT, "v1/x", &[]).unwrap();
        let b = endpoint("", DEFAULT, "/v1/x", &[]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn endpoint_keeps_colon_in_path() {
        let url = endpoint(
            "",
            DEFAULT,
            "/v1beta/models/gemini-3.8-flash:generateContent",
            &[],
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.example.com/v1beta/models/gemini-3.8-flash:generateContent"
        );
    }

    #[test]
    fn endpoint_without_query_has_no_question_mark() {
        let url = endpoint("https://h.example.com/p?old=1", DEFAULT, "/v1/x", &[]).unwrap();
        assert_eq!(url.query(), None);
        assert!(!url.as_str().contains('?'));
    }

    #[test]
    fn endpoint_query_pairs_are_form_encoded() {
        let url = endpoint("", DEFAULT, "/v1/x", &[("q", "a b&c+d"), ("a", "1")]).unwrap();
        assert_eq!(url.query(), Some("q=a+b%26c%2Bd&a=1"));
    }
}
