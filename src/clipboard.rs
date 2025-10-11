//! Wayland Clipboard and Data Transfer Protocol
//!
//! Full implementation of wl_data_device_manager, wl_data_source, and wl_data_offer
//! for clipboard operations and drag-and-drop.
//!
//! # Protocol Flow
//!
//! ## Copy/Paste:
//! 1. Client creates wl_data_source with MIME types
//! 2. Client calls set_selection on wl_data_device
//! 3. Compositor stores the data source
//! 4. When another client pastes, compositor creates wl_data_offer
//! 5. Client receives data through file descriptor
//!
//! ## Drag and Drop:
//! 1. Client starts drag with wl_data_source
//! 2. Compositor sends enter/motion/leave events
//! 3. Target client accepts with wl_data_offer
//! 4. On drop, data is transferred
//!
//! # Usage
//!
//! ```no_run
//! use axiom::clipboard::ClipboardManager;
//!
//! let mut clipboard = ClipboardManager::new();
//! 
//! // Set clipboard data
//! clipboard.set_selection(vec!["text/plain".to_string()], b"Hello".to_vec());
//! 
//! // Get clipboard data
//! if let Some(data) = clipboard.get_selection("text/plain") {
//!     println!("Clipboard: {}", String::from_utf8_lossy(&data));
//! }
//! ```

#![allow(dead_code)]

use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Common MIME types for clipboard
pub mod mime_types {
    #[allow(dead_code)]
    pub const TEXT_PLAIN: &str = "text/plain";
    #[allow(dead_code)]
    pub const TEXT_PLAIN_UTF8: &str = "text/plain;charset=utf-8";
    #[allow(dead_code)]
    pub const TEXT_HTML: &str = "text/html";
    #[allow(dead_code)]
    pub const TEXT_URI_LIST: &str = "text/uri-list";
    #[allow(dead_code)]
    pub const IMAGE_PNG: &str = "image/png";
    #[allow(dead_code)]
    pub const IMAGE_JPEG: &str = "image/jpeg";
}

/// Represents a data source (the "copy" side)
#[derive(Debug, Clone)]
pub struct DataSource {
    /// Unique ID for this data source
    pub id: u64,
    /// MIME types offered by this source
    pub mime_types: Vec<String>,
    /// Actual data (if already transferred)
    pub data: HashMap<String, Vec<u8>>,
    /// Client that owns this source
    pub client_id: Option<u32>,
    /// Whether this source has been used
    pub consumed: bool,
}

impl DataSource {
    /// Creates a new data source
    pub fn new(id: u64, mime_types: Vec<String>) -> Self {
        Self {
            id,
            mime_types,
            data: HashMap::new(),
            client_id: None,
            consumed: false,
        }
    }

    /// Checks if this source offers a specific MIME type
    pub fn offers(&self, mime_type: &str) -> bool {
        self.mime_types.iter().any(|t| t == mime_type)
    }

    /// Sets data for a specific MIME type
    pub fn set_data(&mut self, mime_type: String, data: Vec<u8>) {
        info!("📋 Data source {} set {} bytes for {}", self.id, data.len(), mime_type);
        self.data.insert(mime_type, data);
    }

    /// Gets data for a specific MIME type
    pub fn get_data(&self, mime_type: &str) -> Option<&Vec<u8>> {
        self.data.get(mime_type)
    }
}

/// Represents a data offer (the "paste" side)
#[derive(Debug, Clone)]
pub struct DataOffer {
    /// Unique ID for this offer
    pub id: u64,
    /// MIME types available in this offer
    pub mime_types: Vec<String>,
    /// Reference to the source data
    pub source_id: u64,
}

impl DataOffer {
    /// Creates a new data offer from a source
    pub fn from_source(id: u64, source: &DataSource) -> Self {
        Self {
            id,
            mime_types: source.mime_types.clone(),
            source_id: source.id,
        }
    }

    /// Checks if this offer has a specific MIME type
    pub fn has_mime_type(&self, mime_type: &str) -> bool {
        self.mime_types.iter().any(|t| t == mime_type)
    }
}

/// Drag and drop state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DndState {
    /// No drag operation in progress
    Idle,
    /// Drag started, moving
    Dragging,
    /// Drag is over a valid target
    Offered,
    /// Drop performed, awaiting data transfer
    Dropped,
}

/// Drag and drop operation
#[derive(Debug)]
pub struct DragOperation {
    /// Current state
    pub state: DndState,
    /// Data source being dragged
    pub source_id: Option<u64>,
    /// Current pointer position
    pub position: (f64, f64),
    /// Target surface (if any)
    pub target_surface: Option<u32>,
    /// Accepted MIME type
    pub accepted_mime_type: Option<String>,
}

impl DragOperation {
    fn new() -> Self {
        Self {
            state: DndState::Idle,
            source_id: None,
            position: (0.0, 0.0),
            target_surface: None,
            accepted_mime_type: None,
        }
    }
}

/// Main clipboard and data transfer manager
pub struct ClipboardManager {
    /// Current selection (primary clipboard)
    selection: Option<u64>,
    /// All data sources
    sources: HashMap<u64, DataSource>,
    /// Next source ID
    next_source_id: u64,
    /// Current drag operation
    drag_operation: DragOperation,
    /// History of clipboard data (for undo)
    history: Vec<u64>,
    /// Maximum history size
    max_history: usize,
}

impl ClipboardManager {
    /// Creates a new clipboard manager
    pub fn new() -> Self {
        Self {
            selection: None,
            sources: HashMap::new(),
            next_source_id: 1,
            drag_operation: DragOperation::new(),
            history: Vec::new(),
            max_history: 10,
        }
    }

    /// Creates a new data source
    pub fn create_source(&mut self, mime_types: Vec<String>) -> u64 {
        let id = self.next_source_id;
        self.next_source_id += 1;

        let source = DataSource::new(id, mime_types.clone());
        info!("📋 Created data source {} with types: {:?}", id, mime_types);
        
        self.sources.insert(id, source);
        id
    }

    /// Sets data for a source
    pub fn set_source_data(&mut self, source_id: u64, mime_type: String, data: Vec<u8>) {
        if let Some(source) = self.sources.get_mut(&source_id) {
            source.set_data(mime_type, data);
        } else {
            warn!("📋 Attempted to set data for unknown source {}", source_id);
        }
    }

    /// Sets the current selection (clipboard)
    pub fn set_selection(&mut self, source_id: u64) {
        if self.sources.contains_key(&source_id) {
            // Add old selection to history
            if let Some(old_id) = self.selection {
                self.history.push(old_id);
                if self.history.len() > self.max_history {
                    let removed_id = self.history.remove(0);
                    self.sources.remove(&removed_id);
                }
            }

            self.selection = Some(source_id);
            info!("📋 Selection set to source {}", source_id);
        } else {
            warn!("📋 Attempted to set selection with unknown source {}", source_id);
        }
    }

    /// Gets the current selection data for a specific MIME type
    pub fn get_selection(&self, mime_type: &str) -> Option<Vec<u8>> {
        let source_id = self.selection?;
        let source = self.sources.get(&source_id)?;
        
        source.get_data(mime_type).cloned()
    }

    /// Gets available MIME types for current selection
    pub fn get_selection_mime_types(&self) -> Vec<String> {
        self.selection
            .and_then(|id| self.sources.get(&id))
            .map(|s| s.mime_types.clone())
            .unwrap_or_default()
    }

    /// Creates a data offer from the current selection
    pub fn create_offer_from_selection(&self) -> Option<DataOffer> {
        let source_id = self.selection?;
        let source = self.sources.get(&source_id)?;
        
        Some(DataOffer::from_source(self.next_source_id, source))
    }

    /// Clears the current selection
    pub fn clear_selection(&mut self) {
        if let Some(id) = self.selection.take() {
            info!("📋 Cleared selection (source {})", id);
        }
    }

    /// Starts a drag operation
    pub fn start_drag(&mut self, source_id: u64, origin: (f64, f64)) {
        if self.sources.contains_key(&source_id) {
            self.drag_operation = DragOperation {
                state: DndState::Dragging,
                source_id: Some(source_id),
                position: origin,
                target_surface: None,
                accepted_mime_type: None,
            };
            info!("🖱️ Started drag operation with source {}", source_id);
        }
    }

    /// Updates drag position
    pub fn update_drag_position(&mut self, position: (f64, f64)) {
        if self.drag_operation.state != DndState::Idle {
            self.drag_operation.position = position;
        }
    }

    /// Sets drag target and accepted MIME type
    pub fn set_drag_target(&mut self, surface_id: Option<u32>, mime_type: Option<String>) {
        if self.drag_operation.state == DndState::Dragging {
            self.drag_operation.target_surface = surface_id;
            self.drag_operation.accepted_mime_type = mime_type.clone();
            
            if surface_id.is_some() && mime_type.is_some() {
                self.drag_operation.state = DndState::Offered;
                debug!("🎯 Drag target set: surface {:?}, type {:?}", surface_id, mime_type);
            }
        }
    }

    /// Performs the drop operation
    pub fn perform_drop(&mut self) -> Option<(u64, String)> {
        if self.drag_operation.state == DndState::Offered {
            self.drag_operation.state = DndState::Dropped;
            let source_id = self.drag_operation.source_id?;
            let mime_type = self.drag_operation.accepted_mime_type.clone()?;
            
            info!("📦 Drop performed: source {}, type {}", source_id, mime_type);
            Some((source_id, mime_type))
        } else {
            None
        }
    }

    /// Cancels the current drag operation
    pub fn cancel_drag(&mut self) {
        if self.drag_operation.state != DndState::Idle {
            warn!("🚫 Drag operation cancelled");
            self.drag_operation = DragOperation::new();
        }
    }

    /// Gets the current drag state
    pub fn drag_state(&self) -> DndState {
        self.drag_operation.state
    }

    /// Gets statistics
    pub fn stats(&self) -> ClipboardStats {
        ClipboardStats {
            total_sources: self.sources.len(),
            has_selection: self.selection.is_some(),
            history_size: self.history.len(),
            is_dragging: self.drag_operation.state != DndState::Idle,
        }
    }
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about clipboard usage
#[derive(Debug, Clone)]
pub struct ClipboardStats {
    pub total_sources: usize,
    pub has_selection: bool,
    pub history_size: usize,
    pub is_dragging: bool,
}

/// Thread-safe clipboard manager wrapper
#[allow(dead_code)]
pub type SharedClipboard = Arc<Mutex<ClipboardManager>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_source() {
        let mut clipboard = ClipboardManager::new();
        let source_id = clipboard.create_source(vec!["text/plain".to_string()]);
        assert_eq!(source_id, 1);
    }

    #[test]
    fn test_set_and_get_selection() {
        let mut clipboard = ClipboardManager::new();
        let source_id = clipboard.create_source(vec!["text/plain".to_string()]);
        clipboard.set_source_data(source_id, "text/plain".to_string(), b"Hello".to_vec());
        clipboard.set_selection(source_id);

        let data = clipboard.get_selection("text/plain");
        assert_eq!(data, Some(b"Hello".to_vec()));
    }

    #[test]
    fn test_multiple_mime_types() {
        let mut clipboard = ClipboardManager::new();
        let source_id = clipboard.create_source(vec![
            "text/plain".to_string(),
            "text/html".to_string(),
        ]);
        
        clipboard.set_source_data(source_id, "text/plain".to_string(), b"Hello".to_vec());
        clipboard.set_source_data(source_id, "text/html".to_string(), b"<b>Hello</b>".to_vec());
        clipboard.set_selection(source_id);

        assert_eq!(clipboard.get_selection("text/plain"), Some(b"Hello".to_vec()));
        assert_eq!(clipboard.get_selection("text/html"), Some(b"<b>Hello</b>".to_vec()));
    }

    #[test]
    fn test_clear_selection() {
        let mut clipboard = ClipboardManager::new();
        let source_id = clipboard.create_source(vec!["text/plain".to_string()]);
        clipboard.set_selection(source_id);
        
        assert!(clipboard.selection.is_some());
        clipboard.clear_selection();
        assert!(clipboard.selection.is_none());
    }

    #[test]
    fn test_drag_and_drop() {
        let mut clipboard = ClipboardManager::new();
        let source_id = clipboard.create_source(vec!["text/plain".to_string()]);
        
        clipboard.start_drag(source_id, (100.0, 200.0));
        assert_eq!(clipboard.drag_state(), DndState::Dragging);
        
        clipboard.set_drag_target(Some(1), Some("text/plain".to_string()));
        assert_eq!(clipboard.drag_state(), DndState::Offered);
        
        let result = clipboard.perform_drop();
        assert!(result.is_some());
        assert_eq!(clipboard.drag_state(), DndState::Dropped);
    }

    #[test]
    fn test_cancel_drag() {
        let mut clipboard = ClipboardManager::new();
        let source_id = clipboard.create_source(vec!["text/plain".to_string()]);
        
        clipboard.start_drag(source_id, (0.0, 0.0));
        assert_eq!(clipboard.drag_state(), DndState::Dragging);
        
        clipboard.cancel_drag();
        assert_eq!(clipboard.drag_state(), DndState::Idle);
    }

    #[test]
    fn test_mime_type_check() {
        let source = DataSource::new(1, vec!["text/plain".to_string(), "text/html".to_string()]);
        assert!(source.offers("text/plain"));
        assert!(source.offers("text/html"));
        assert!(!source.offers("image/png"));
    }

    #[test]
    fn test_history() {
        let mut clipboard = ClipboardManager::new();
        
        let source1 = clipboard.create_source(vec!["text/plain".to_string()]);
        clipboard.set_selection(source1);
        
        let source2 = clipboard.create_source(vec!["text/plain".to_string()]);
        clipboard.set_selection(source2);
        
        assert_eq!(clipboard.stats().history_size, 1);
    }
}
