//! Minimal clipboard and drag-and-drop scaffolding
//! This module provides stubs for clipboard and DnD interactions. Real protocol
//! wiring will be added in a future phase.

use log::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct ClipboardManager {
    selection: Option<String>,
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self { selection: None }
    }

    pub fn set_selection(&mut self, data: String) {
        info!("📋 Clipboard selection set ({} bytes)", data.len());
        self.selection = Some(data);
    }

    pub fn get_selection(&self) -> Option<String> {
        debug!("📋 Clipboard selection queried");
        self.selection.clone()
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        info!("📋 Clipboard cleared");
        self.selection = None;
    }
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DragAndDropManager {}

#[allow(dead_code)]
impl DragAndDropManager {
    pub fn new() -> Self {
        Self {}
    }

    pub fn start_drag(&self, mime_types: &[&str]) {
        info!("🖱️ DnD start with types: {:?}", mime_types);
    }

    pub fn offer(&self, mime_type: &str) {
        debug!("🧾 DnD offer: {}", mime_type);
    }

    pub fn accept(&self, mime_type: &str) {
        info!("✅ DnD accepted: {}", mime_type);
    }

    pub fn drop_perform(&self) {
        info!("📦 DnD drop performed");
    }

    pub fn cancel(&self) {
        warn!("🚫 DnD cancelled");
    }
}

impl Default for DragAndDropManager {
    fn default() -> Self {
        Self::new()
    }
}
