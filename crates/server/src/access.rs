use std::{
    fmt::Write,
    net::{IpAddr, SocketAddr},
};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

const CAPABILITY_BYTES: usize = 32;

#[derive(Clone)]
pub(crate) struct AccessPolicy {
    authorities: [String; 2],
    origins: [String; 2],
    capability: String,
}

impl AccessPolicy {
    pub(crate) fn generate(address: SocketAddr) -> Result<Self, getrandom::Error> {
        let mut bytes = [0_u8; CAPABILITY_BYTES];
        getrandom::fill(&mut bytes)?;
        let mut capability = String::with_capacity(CAPABILITY_BYTES * 2);
        for byte in bytes {
            write!(capability, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Ok(Self::new(address, capability))
    }

    #[cfg(test)]
    pub(crate) fn for_test(address: SocketAddr, capability: &str) -> Self {
        Self::new(address, capability.to_owned())
    }

    fn new(address: SocketAddr, capability: String) -> Self {
        let numeric_host = match address.ip() {
            IpAddr::V4(address) => address.to_string(),
            IpAddr::V6(address) => format!("[{address}]"),
        };
        let (numeric, localhost) = if address.port() == 80 {
            (numeric_host, "localhost".to_owned())
        } else {
            (
                format!("{numeric_host}:{}", address.port()),
                format!("localhost:{}", address.port()),
            )
        };
        let origins = [format!("http://{numeric}"), format!("http://{localhost}")];
        Self {
            authorities: [numeric, localhost],
            origins,
            capability,
        }
    }

    pub(crate) fn startup_url(&self) -> String {
        format!("{}/#{}", self.origins[0], self.capability)
    }

    fn allows_authority(&self, authority: Option<&header::HeaderValue>) -> bool {
        authority
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| self.authorities.iter().any(|allowed| value == allowed))
    }

    fn allows_origin(&self, origin: Option<&header::HeaderValue>) -> bool {
        origin.is_none_or(|value| {
            value
                .to_str()
                .ok()
                .is_some_and(|value| self.origins.iter().any(|allowed| value == allowed))
        })
    }

    fn allows_bearer(&self, authorization: Option<&header::HeaderValue>) -> bool {
        authorization
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|value| constant_time_eq(value.as_bytes(), self.capability.as_bytes()))
    }

    fn allows_query_capability(&self, query: Option<&str>) -> bool {
        let mut tokens = query
            .unwrap_or_default()
            .split('&')
            .filter_map(|pair| pair.strip_prefix("access_token="));
        let Some(token) = tokens.next() else {
            return false;
        };
        tokens.next().is_none() && constant_time_eq(token.as_bytes(), self.capability.as_bytes())
    }
}

pub(crate) async fn enforce(policy: AccessPolicy, request: Request<Body>, next: Next) -> Response {
    let response = if !policy.allows_authority(request.headers().get(header::HOST)) {
        StatusCode::MISDIRECTED_REQUEST.into_response()
    } else if !request.uri().path().starts_with("/api/") {
        next.run(request).await
    } else if !policy.allows_origin(request.headers().get(header::ORIGIN)) {
        StatusCode::FORBIDDEN.into_response()
    } else {
        let allowed = if is_terminal_route(request.uri().path()) {
            policy.allows_query_capability(request.uri().query())
        } else {
            policy.allows_bearer(request.headers().get(header::AUTHORIZATION))
        };
        if allowed {
            next.run(request).await
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        }
    };
    with_browser_security_headers(response)
}

fn with_browser_security_headers(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static("frame-ancestors 'none'"),
    );
    response.headers_mut().insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("DENY"),
    );
    response
}

fn is_terminal_route(path: &str) -> bool {
    let Some(id) = path
        .strip_prefix("/api/environments/")
        .and_then(|rest| rest.strip_suffix("/terminal"))
    else {
        return false;
    };
    !id.is_empty() && !id.contains('/')
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&left, &right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_url_uses_numeric_loopback_authority_and_fragment() {
        let policy = AccessPolicy::for_test("[::1]:4321".parse().unwrap(), "secret");

        assert_eq!(policy.startup_url(), "http://[::1]:4321/#secret");
    }

    #[test]
    fn default_http_port_uses_browser_canonical_authorities() {
        let ipv4 = AccessPolicy::for_test("127.0.0.1:80".parse().unwrap(), "secret");
        let ipv6 = AccessPolicy::for_test("[::1]:80".parse().unwrap(), "secret");

        assert_eq!(ipv4.startup_url(), "http://127.0.0.1/#secret");
        assert_eq!(ipv6.startup_url(), "http://[::1]/#secret");
        assert!(ipv4.allows_authority(Some(&header::HeaderValue::from_static("localhost"))));
        assert!(ipv6.allows_origin(Some(&header::HeaderValue::from_static("http://[::1]"))));
    }

    #[test]
    fn terminal_route_matching_is_exact() {
        assert!(is_terminal_route("/api/environments/env-00000001/terminal"));
        assert!(!is_terminal_route("/api/environments//terminal"));
        assert!(!is_terminal_route("/api/environments/env/extra/terminal"));
        assert!(!is_terminal_route(
            "/api/environments/env-00000001/observations"
        ));
    }
}
