use serde::{Deserialize, Serialize};

/// A protocol version. Both sides must share the major; a minor may add
/// fields the other side ignores.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Changes when a peer on the old major could misread a frame.
    pub major: u16,
    /// Changes when fields are only added.
    pub minor: u16,
}

/// One request frame: the sender's version, an id the reply repeats, and the
/// caller's body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request<B> {
    /// The sender's protocol version.
    pub v: Version,
    /// Chosen by the client and repeated in the reply.
    pub id: String,
    /// The caller's payload.
    pub body: B,
}

/// One response frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response<R> {
    /// The server's protocol version.
    pub v: Version,
    /// The id of the request this answers.
    pub id: String,
    /// The handler's result.
    #[serde(flatten)]
    pub outcome: Outcome<R>,
}

/// The handler's result, as `{"ok": …}` or `{"err": …}` on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome<R> {
    /// The handler's response.
    Ok(R),
    /// The handler's error, or one of the transport's own codes.
    Err(Fault),
}

/// An error on the wire. The caller owns the codes, except two the server
/// sends itself: `unsupported_version` for another major, and
/// `invalid_request` for a body the handler's request type cannot decode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fault {
    /// A stable code the client matches on.
    pub code: String,
    /// Text for a person.
    pub message: String,
    /// Whether the same request may succeed later.
    pub retryable: bool,
    /// Structured context the client may match on, such as an upstream
    /// status. The handler decides what goes here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}
