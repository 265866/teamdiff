/// A finished request. `status` is 0 when the page couldn't read the response, as with
/// Cloudflare's block page, which carries no CORS headers, or when the page itself was refused;
/// `body` then says why.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// The HTTP status, or 0 when the page couldn't read the response.
    pub status: u16,
    /// The `Retry-After` header of a 429 or 503 answer, which is read from the full response,
    /// since tracker.gg doesn't expose it to the page.
    pub retry_after: Option<String>,
    /// The body, cut down by the request's trim when the request succeeded, or why it failed.
    pub body: String,
}
