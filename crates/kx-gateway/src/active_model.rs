//! Model Control v2 — the host active-default-model control ([`ActiveModelControl`]).
//!
//! An OFF-JOURNAL advisory hint (SN-8): it authorizes nothing and the server never
//! re-routes `kx/recipes/chat`. It exists so the default a client chats with is
//! switchable from CLI/SDK. Validated against the LIVE served catalog (the same shared
//! entries handle the model catalog + puller hold), so a runtime-pulled model is a
//! valid active target. Rebuilds empty (→ primary) on restart.

use std::sync::{Arc, PoisonError, RwLock};

use kx_gateway_core::{ActiveModelControl, GatewayError, ModelSummaryEntry};

/// The host active-default-model control. `None` ⇒ the primary (no override).
pub(crate) struct HostActiveModel {
    active: RwLock<Option<String>>,
    /// The shared served-catalog entries (validation: an active model MUST be served).
    catalog: Arc<RwLock<Vec<ModelSummaryEntry>>>,
}

impl HostActiveModel {
    pub(crate) fn new(catalog: Arc<RwLock<Vec<ModelSummaryEntry>>>) -> Self {
        Self {
            active: RwLock::new(None),
            catalog,
        }
    }

    fn is_served(&self, id: &str) -> bool {
        self.catalog
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|e| e.model_id == id)
    }
}

impl ActiveModelControl for HostActiveModel {
    fn get(&self) -> Option<String> {
        self.active
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, model_id: &str) -> Result<Option<String>, GatewayError> {
        let id = model_id.trim();
        if id.is_empty() {
            // An empty id CLEARS the override (back to the primary).
            *self.active.write().unwrap_or_else(PoisonError::into_inner) = None;
            return Ok(None);
        }
        // Fail-closed: never set an unrouteable active model.
        if !self.is_served(id) {
            return Err(GatewayError::NotFound("model not in the served catalog"));
        }
        *self.active.write().unwrap_or_else(PoisonError::into_inner) = Some(id.to_string());
        Ok(Some(id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> ModelSummaryEntry {
        ModelSummaryEntry {
            model_id: id.into(),
            modalities: vec!["text".into()],
            description: id.into(),
            serving: false,
            context_len: 4096,
            loaded: false,
            chat_handle: format!("kx/recipes/m-{id}"),
            engine: "kx-ollama".into(),
            can_embed: false,
            source: "ollama".into(),
            active: false,
            chat_rag_handle: String::new(),
        }
    }

    #[test]
    fn set_active_validates_against_the_served_catalog() {
        let catalog = Arc::new(RwLock::new(vec![entry("a"), entry("b")]));
        let am = HostActiveModel::new(catalog.clone());
        assert!(am.get().is_none());
        assert_eq!(am.set("b").unwrap(), Some("b".to_string()));
        assert_eq!(am.get(), Some("b".to_string()));
        // Clearing returns to the primary (None).
        assert_eq!(am.set("").unwrap(), None);
        assert!(am.get().is_none());
        // An unserved id is fail-closed NotFound (the previous selection is preserved).
        am.set("a").unwrap();
        assert!(matches!(
            am.set("nope").unwrap_err(),
            GatewayError::NotFound(_)
        ));
        assert_eq!(am.get(), Some("a".to_string()));
    }

    #[test]
    fn active_validates_a_runtime_registered_model() {
        // A pulled model appended to the shared catalog becomes a valid active target.
        let catalog = Arc::new(RwLock::new(vec![entry("a")]));
        let am = HostActiveModel::new(catalog.clone());
        assert!(am.set("pulled").is_err());
        catalog
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry("pulled"));
        assert_eq!(am.set("pulled").unwrap(), Some("pulled".to_string()));
    }
}
