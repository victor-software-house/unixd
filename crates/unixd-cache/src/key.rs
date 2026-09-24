use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest as _, Sha256};

use crate::Error;

/// Version of the entry file layout. It is hashed into every key, so changing
/// the layout starts from an empty cache instead of misreading old files.
pub(crate) const FORMAT: u8 = 1;

/// Name segments that mark a credential or a session.
const FORBIDDEN_WORDS: &[&str] = &[
    "token",
    "tokens",
    "secret",
    "secrets",
    "password",
    "passwords",
    "passwd",
    "credential",
    "credentials",
    "authorization",
    "bearer",
    "cookie",
    "apikey",
];

/// Compounds that mark a credential. Each is matched at the start or the end
/// of any run of adjacent words with the separators removed, so `access_key`,
/// `x-api-key`, `ACCESSKEY`, and `oauthaccesstoken` match while
/// `profession_id` does not match `sessionid`. End matching also refuses
/// `possession_id`, a deliberate false positive, and a compound in the middle
/// of one word, such as `myapikeyvalue`, is not matched.
const FORBIDDEN_SUBSTRINGS: &[&str] = &[
    "apikey",
    "accesskey",
    "privatekey",
    "secretkey",
    "accesstoken",
    "authtoken",
    "apitoken",
    "bearertoken",
    "idtoken",
    "refreshtoken",
    "sessiontoken",
    "csrftoken",
    "xsrftoken",
    "sessionid",
    "clientsecret",
    "passphrase",
];

/// Word endings that mark a credential run together with a qualifier, such
/// as `jwttoken` or `githubsecret`. A word that only starts with one, such
/// as `tokenizer` or `secretary`, passes.
const FORBIDDEN_ENDINGS: &[&str] = &[
    "token",
    "tokens",
    "secret",
    "secrets",
    "password",
    "passwords",
    "passwd",
    "credential",
    "credentials",
];

/// Whole names that describe how or where a result is shown, or which request
/// asked for it, never what was fetched.
const FORBIDDEN_NAMES: &[&str] = &[
    "format",
    "output",
    "output_format",
    "destination",
    "dest",
    "path",
    "request_id",
    "requestid",
];

/// A cache key: a namespace and a digest of named parts.
///
/// Two keys are equal when their namespaces and parts are equal, whatever
/// order the parts were added in.
#[derive(Clone, PartialEq, Eq)]
pub struct Key {
    namespace: String,
    digest: String,
}

impl Key {
    /// Starts a key in `namespace`, for example the kind of request.
    #[must_use]
    pub fn builder(namespace: impl Into<String>) -> KeyBuilder {
        KeyBuilder {
            namespace: namespace.into(),
            parts: BTreeMap::new(),
        }
    }

    /// The namespace the key was built in.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The hex SHA-256 digest that names the entry on disk.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Key")
            .field("namespace", &self.namespace)
            .field("digest", &self.digest)
            .finish()
    }
}

/// Collects the named parts of a [`Key`].
#[derive(Debug)]
pub struct KeyBuilder {
    namespace: String,
    parts: BTreeMap<String, String>,
}

impl KeyBuilder {
    /// Adds one named part, such as the query or the normalized URL.
    ///
    /// Cache identity is what was fetched, never who asked or how the result
    /// will be shown. A name that marks a credential (`token`, `secret`,
    /// `api_key`, `authorization`, …), an output format, a destination path, or
    /// a request id is refused. Credential words match whole or at the end of
    /// a word, so `jwttoken` is refused while `tokenizer` and `secretary`
    /// pass. Only the name is
    /// checked: a credential under an innocent name is the caller's bug.
    ///
    /// # Errors
    ///
    /// [`Error::ForbiddenKeyPart`] for a refused name, and
    /// [`Error::DuplicateKeyPart`] when the name was already added.
    pub fn part(mut self, name: &str, value: impl Into<String>) -> Result<Self, Error> {
        if forbidden(name) {
            return Err(Error::ForbiddenKeyPart(name.to_owned()));
        }
        if self.parts.insert(name.to_owned(), value.into()).is_some() {
            return Err(Error::DuplicateKeyPart(name.to_owned()));
        }
        Ok(self)
    }

    /// Hashes the namespace and the sorted parts into a [`Key`].
    #[must_use]
    pub fn build(self) -> Key {
        let mut hasher = Sha256::new();
        hasher.update([FORMAT]);
        feed(&mut hasher, &self.namespace);
        for (name, value) in &self.parts {
            feed(&mut hasher, name);
            feed(&mut hasher, value);
        }
        Key {
            digest: hex(&hasher.finalize()),
            namespace: self.namespace,
        }
    }
}

/// Length-prefixes each field, so `("ab", "c")` and `("a", "bc")` differ.
fn feed(hasher: &mut Sha256, field: &str) {
    hasher.update((field.len() as u64).to_le_bytes());
    hasher.update(field.as_bytes());
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Splits camelCase before lowercasing, so `accessToken` is checked as
/// `access_token` and `APIToken` as `api_token`.
///
/// A boundary goes before an uppercase letter that follows a lowercase letter
/// or digit, or that starts a capitalized word after an acronym. A run of
/// capitals such as `APIKEY` stays whole. A boundary also goes before a
/// digit that follows a letter, so `token2` is checked as `token_2`.
fn forbidden(name: &str) -> bool {
    let characters: Vec<char> = name.chars().collect();
    let mut split = String::with_capacity(name.len() + 4);
    for (index, &character) in characters.iter().enumerate() {
        if index > 0 && character.is_ascii_digit() && characters[index - 1].is_ascii_alphabetic() {
            split.push('_');
        }
        if index > 0 && character.is_ascii_uppercase() {
            let previous = characters[index - 1];
            let next_lower = characters
                .get(index + 1)
                .is_some_and(char::is_ascii_lowercase);
            if previous.is_ascii_lowercase()
                || previous.is_ascii_digit()
                || (previous.is_ascii_uppercase() && next_lower)
            {
                split.push('_');
            }
        }
        split.push(character);
    }
    let name: String = split
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    let words: Vec<&str> = name.split('_').filter(|word| !word.is_empty()).collect();
    FORBIDDEN_NAMES.contains(&name.as_str())
        || words.iter().any(|word| {
            FORBIDDEN_WORDS.contains(word)
                || FORBIDDEN_ENDINGS
                    .iter()
                    .any(|ending| word.ends_with(ending))
        })
        || (0..words.len()).any(|start| {
            (start..words.len()).any(|end| {
                let span = words[start..=end].concat();
                FORBIDDEN_SUBSTRINGS
                    .iter()
                    .any(|compound| span.starts_with(compound) || span.ends_with(compound))
            })
        })
}
