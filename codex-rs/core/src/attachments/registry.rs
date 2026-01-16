//! Attachment registry for collecting attachments.

use codex_protocol::models::AttachmentData;
use futures::future::BoxFuture;
use futures::future::join_all;

use crate::codex::Session;
use crate::codex::TurnContext;

/// Collector function signature.
///
/// Each collector is a function that takes session and turn context,
/// and returns a future resolving to a vector of attachment data.
pub type CollectFn = for<'a> fn(&'a Session, &'a TurnContext) -> BoxFuture<'a, Vec<AttachmentData>>;

/// Registry of attachment collectors.
///
/// Collectors are executed in parallel when `collect_all` is called.
#[derive(Default)]
pub struct AttachmentRegistry {
    collectors: Vec<CollectFn>,
}

impl AttachmentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a collector function.
    pub fn register(&mut self, collector: CollectFn) -> &mut Self {
        self.collectors.push(collector);
        self
    }

    /// Collect all attachments from registered collectors.
    ///
    /// Collectors are executed in parallel and results are flattened.
    pub async fn collect_all(&self, session: &Session, turn: &TurnContext) -> Vec<AttachmentData> {
        join_all(self.collectors.iter().map(|f| f(session, turn)))
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// Check if registry has any collectors.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.collectors.is_empty()
    }

    /// Get the number of registered collectors.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.collectors.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new() {
        let registry = AttachmentRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }
}
