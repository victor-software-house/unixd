use std::collections::BTreeMap;
use std::fmt;

use sha2::{Digest as _, Sha256};

use crate::Error;

/// Entry file layout version. Hashed into every key, so a layout change
/// misses old files instead of misreading them.
pub(crate) const FORMAT: u8 = 1;

/// A namespace and a digest of named parts. Part order does not matter.
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
    /// # Errors
    ///
    /// [`Error::DuplicateKeyPart`] when the name was already added.
    pub fn part(mut self, name: &str, value: impl Into<String>) -> Result<Self, Error> {
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
