//! HTTP client patterns (Category::Http). curl, wget, httpie.

use crate::registry::Pattern;

pub const CURL: Pattern = Pattern {
    name: "curl",
    category: crate::category::Category::Http,
    matchers: &["curl"],
    // Keep status line + headers; drop the body (the response body is usually the bulk).
    keep: Some(&[
        "HTTP/",
        "Content-Type:",
        "Content-Length:",
        "Set-Cookie:",
        "Location:",
        "Server:",
        "< HTTP/",
        "< Content-",
        "< Location:",
        "* ",
    ]),
    drop: None,
};
