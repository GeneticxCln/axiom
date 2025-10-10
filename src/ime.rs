//! Input Method Editor (IME) Protocol Support
//!
//! Implementation of the zwp_input_method_v2 protocol for input method support,
//! enabling complex text input methods like Chinese, Japanese, Korean, and others.
//!
//! # Protocol Overview
//!
//! The IME protocol provides a communication channel between the compositor,
//! text input clients, and input method applications (like ibus, fcitx, etc.).
//!
//! ## Architecture
//!
//! ```text
//! ┌────────────────┐       ┌──────────────┐       ┌─────────────────┐
//! │  Text Input    │◄─────►│  Compositor  │◄─────►│  Input Method   │
//! │  Client App    │       │   (Axiom)    │       │  (ibus/fcitx)   │
//! └────────────────┘       └──────────────┘       └─────────────────┘
//! ```
//!
//! ## Protocol Flow
//!
//! 1. Text input field gains focus
//! 2. Compositor activates input method
//! 3. User types characters
//! 4. Input method processes keystrokes
//! 5. Input method sends preedit (candidate) text
//! 6. User selects composition
//! 7. Input method commits final text
//! 8. Compositor sends text to client
//!
//! # Features
//!
//! - Text input state tracking
//! - Preedit (composition) string management
//! - Content type hints (email, URL, password, etc.)
//! - Text change reason tracking
//! - Cursor position and selection
//! - Commit string handling
//! - Input method lifecycle management
//!
//! # Usage
//!
//! ```no_run
//! use axiom::ime::{ImeManager, ContentHint, ContentPurpose};
//!
//! let mut ime = ImeManager::new();
//!
//! // Activate input method for a client
//! ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);
//!
//! // Send preedit text (composition candidates)
//! ime.set_preedit_string(1, "你好".to_string(), 0, 6);
//!
//! // Commit final text
//! ime.commit_string(1, "你好".to_string());
//!
//! // Deactivate
//! ime.deactivate(1);
//! ```

use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Content purpose hints for the input field
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentPurpose {
    /// Normal text input
    Normal,
    /// Alphabetic characters only
    Alpha,
    /// Digits only
    Digits,
    /// Numeric input
    Number,
    /// Phone number
    Phone,
    /// URL input
    Url,
    /// Email address
    Email,
    /// Name input
    Name,
    /// Password input
    Password,
    /// PIN input
    Pin,
    /// Date input
    Date,
    /// Time input
    Time,
    /// Date and time
    Datetime,
    /// Terminal input
    Terminal,
}

/// Content hint flags (can be combined)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentHint {
    bits: u32,
}

impl ContentHint {
    /// No special hints
    pub const NONE: ContentHint = ContentHint { bits: 0 };
    /// Auto-completion enabled
    pub const COMPLETION: ContentHint = ContentHint { bits: 1 << 0 };
    /// Spell checking enabled
    pub const SPELLCHECK: ContentHint = ContentHint { bits: 1 << 1 };
    /// Auto-capitalization enabled
    pub const AUTO_CAPITALIZATION: ContentHint = ContentHint { bits: 1 << 2 };
    /// Lowercase preference
    pub const LOWERCASE: ContentHint = ContentHint { bits: 1 << 3 };
    /// Uppercase preference
    pub const UPPERCASE: ContentHint = ContentHint { bits: 1 << 4 };
    /// Titlecase preference
    pub const TITLECASE: ContentHint = ContentHint { bits: 1 << 5 };
    /// Hidden text (like password)
    pub const HIDDEN_TEXT: ContentHint = ContentHint { bits: 1 << 6 };
    /// Sensitive data
    pub const SENSITIVE_DATA: ContentHint = ContentHint { bits: 1 << 7 };
    /// Latin text preference
    pub const LATIN: ContentHint = ContentHint { bits: 1 << 8 };
    /// Multiline input
    pub const MULTILINE: ContentHint = ContentHint { bits: 1 << 9 };

    /// Create from raw bits
    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    /// Check if hint has a specific flag
    pub fn has(&self, flag: ContentHint) -> bool {
        (self.bits & flag.bits) != 0
    }

    /// Combine hints
    pub fn or(self, other: ContentHint) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }
}

/// Text change reason
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeReason {
    /// Input method caused the change
    InputMethod,
    /// Other reason (e.g., user action)
    Other,
}

/// Preedit (composition) style
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreeditStyle {
    /// Default style
    Default,
    /// No special highlighting
    None,
    /// Active composition
    Active,
    /// Inactive composition
    Inactive,
    /// Highlight
    Highlight,
    /// Underline
    Underline,
    /// Selection
    Selection,
    /// Incorrect
    Incorrect,
}

/// Text input state for a client
#[derive(Debug, Clone)]
pub struct TextInputState {
    /// Client ID
    pub client_id: u32,
    /// Whether input method is active
    pub active: bool,
    /// Content purpose
    pub content_purpose: ContentPurpose,
    /// Content hints
    pub content_hint: ContentHint,
    /// Current preedit string
    pub preedit_string: String,
    /// Preedit cursor position (in bytes)
    pub preedit_cursor: i32,
    /// Committed text
    pub commit_string: String,
    /// Surrounding text
    pub surrounding_text: String,
    /// Surrounding text cursor position
    pub surrounding_cursor: i32,
    /// Surrounding text anchor (selection end)
    pub surrounding_anchor: i32,
    /// Change reason
    pub change_reason: ChangeReason,
    /// Serial number for state updates
    pub serial: u32,
}

impl TextInputState {
    /// Create a new text input state
    pub fn new(client_id: u32) -> Self {
        Self {
            client_id,
            active: false,
            content_purpose: ContentPurpose::Normal,
            content_hint: ContentHint::NONE,
            preedit_string: String::new(),
            preedit_cursor: 0,
            commit_string: String::new(),
            surrounding_text: String::new(),
            surrounding_cursor: 0,
            surrounding_anchor: 0,
            change_reason: ChangeReason::Other,
            serial: 0,
        }
    }

    /// Check if there's an active preedit
    pub fn has_preedit(&self) -> bool {
        !self.preedit_string.is_empty()
    }

    /// Check if there's surrounding text
    pub fn has_surrounding_text(&self) -> bool {
        !self.surrounding_text.is_empty()
    }

    /// Clear preedit
    pub fn clear_preedit(&mut self) {
        self.preedit_string.clear();
        self.preedit_cursor = 0;
    }

    /// Clear commit
    pub fn clear_commit(&mut self) {
        self.commit_string.clear();
    }
}

/// Input Method Manager
pub struct ImeManager {
    /// Active text input states by client ID
    states: HashMap<u32, TextInputState>,
    /// Currently focused client
    focused_client: Option<u32>,
    /// Statistics
    stats: ImeStats,
}

impl ImeManager {
    /// Create a new IME manager
    pub fn new() -> Self {
        Self {
            states: HashMap::new(),
            focused_client: None,
            stats: ImeStats::default(),
        }
    }

    /// Activate input method for a client
    pub fn activate(
        &mut self,
        client_id: u32,
        purpose: ContentPurpose,
        hint: ContentHint,
    ) -> &mut TextInputState {
        let state = self.states.entry(client_id).or_insert_with(|| {
            self.stats.total_activations += 1;
            TextInputState::new(client_id)
        });

        state.active = true;
        state.content_purpose = purpose;
        state.content_hint = hint;
        state.serial += 1;

        self.focused_client = Some(client_id);

        info!(
            "⌨️  IME activated for client {} (purpose: {:?}, hint: {:?})",
            client_id,
            purpose,
            hint.bits
        );

        state
    }

    /// Deactivate input method for a client
    pub fn deactivate(&mut self, client_id: u32) {
        if let Some(state) = self.states.get_mut(&client_id) {
            state.active = false;
            state.clear_preedit();
            state.clear_commit();
            info!("⌨️  IME deactivated for client {}", client_id);
        }

        if self.focused_client == Some(client_id) {
            self.focused_client = None;
        }
    }

    /// Set preedit (composition) string
    pub fn set_preedit_string(
        &mut self,
        client_id: u32,
        text: String,
        cursor_begin: i32,
        cursor_end: i32,
    ) -> Result<(), String> {
        let state = self
            .states
            .get_mut(&client_id)
            .ok_or("Client not found")?;

        if !state.active {
            return Err("Input method not active".to_string());
        }

        state.preedit_string = text.clone();
        state.preedit_cursor = cursor_begin;
        state.serial += 1;

        debug!(
            "⌨️  Preedit set for client {}: \"{}\" ({}-{})",
            client_id, text, cursor_begin, cursor_end
        );

        self.stats.preedit_updates += 1;
        Ok(())
    }

    /// Commit final text
    pub fn commit_string(&mut self, client_id: u32, text: String) -> Result<(), String> {
        let state = self
            .states
            .get_mut(&client_id)
            .ok_or("Client not found")?;

        if !state.active {
            return Err("Input method not active".to_string());
        }

        state.commit_string = text.clone();
        state.clear_preedit();
        state.serial += 1;

        info!("⌨️  Text committed for client {}: \"{}\"", client_id, text);

        self.stats.text_commits += 1;
        Ok(())
    }

    /// Set surrounding text (text around cursor)
    pub fn set_surrounding_text(
        &mut self,
        client_id: u32,
        text: String,
        cursor: i32,
        anchor: i32,
    ) -> Result<(), String> {
        let state = self
            .states
            .get_mut(&client_id)
            .ok_or("Client not found")?;

        state.surrounding_text = text;
        state.surrounding_cursor = cursor;
        state.surrounding_anchor = anchor;

        debug!(
            "⌨️  Surrounding text updated for client {} (cursor: {}, anchor: {})",
            client_id, cursor, anchor
        );

        Ok(())
    }

    /// Set text change reason
    pub fn set_change_reason(&mut self, client_id: u32, reason: ChangeReason) -> Result<(), String> {
        let state = self
            .states
            .get_mut(&client_id)
            .ok_or("Client not found")?;

        state.change_reason = reason;
        Ok(())
    }

    /// Get text input state for a client
    pub fn get_state(&self, client_id: u32) -> Option<&TextInputState> {
        self.states.get(&client_id)
    }

    /// Get mutable text input state
    pub fn get_state_mut(&mut self, client_id: u32) -> Option<&mut TextInputState> {
        self.states.get_mut(&client_id)
    }

    /// Get currently focused client
    pub fn focused_client(&self) -> Option<u32> {
        self.focused_client
    }

    /// Check if a client has active input method
    pub fn is_active(&self, client_id: u32) -> bool {
        self.states
            .get(&client_id)
            .map(|s| s.active)
            .unwrap_or(false)
    }

    /// Remove client state
    pub fn remove_client(&mut self, client_id: u32) {
        if self.states.remove(&client_id).is_some() {
            info!("⌨️  Removed IME state for client {}", client_id);

            if self.focused_client == Some(client_id) {
                self.focused_client = None;
            }
        }
    }

    /// Get statistics
    pub fn stats(&self) -> ImeStats {
        let mut stats = self.stats;
        stats.active_clients = self.states.values().filter(|s| s.active).count();
        stats
    }
}

impl Default for ImeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about IME usage
#[derive(Debug, Clone, Copy, Default)]
pub struct ImeStats {
    /// Total number of activations
    pub total_activations: usize,
    /// Number of preedit updates
    pub preedit_updates: usize,
    /// Number of text commits
    pub text_commits: usize,
    /// Currently active clients
    pub active_clients: usize,
}

/// Thread-safe IME manager wrapper
pub type SharedIme = Arc<Mutex<ImeManager>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_hint() {
        let hint = ContentHint::COMPLETION.or(ContentHint::SPELLCHECK);
        assert!(hint.has(ContentHint::COMPLETION));
        assert!(hint.has(ContentHint::SPELLCHECK));
        assert!(!hint.has(ContentHint::HIDDEN_TEXT));
    }

    #[test]
    fn test_activate_deactivate() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);

        assert!(ime.is_active(1));
        assert_eq!(ime.focused_client(), Some(1));

        ime.deactivate(1);
        assert!(!ime.is_active(1));
        assert_eq!(ime.focused_client(), None);
    }

    #[test]
    fn test_preedit() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);

        ime.set_preedit_string(1, "你好".to_string(), 0, 6)
            .unwrap();

        let state = ime.get_state(1).unwrap();
        assert!(state.has_preedit());
        assert_eq!(state.preedit_string, "你好");
    }

    #[test]
    fn test_commit() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);

        ime.set_preedit_string(1, "你好".to_string(), 0, 6)
            .unwrap();
        ime.commit_string(1, "你好".to_string()).unwrap();

        let state = ime.get_state(1).unwrap();
        assert_eq!(state.commit_string, "你好");
        assert!(!state.has_preedit());
    }

    #[test]
    fn test_surrounding_text() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);

        ime.set_surrounding_text(1, "Hello world".to_string(), 5, 11)
            .unwrap();

        let state = ime.get_state(1).unwrap();
        assert!(state.has_surrounding_text());
        assert_eq!(state.surrounding_cursor, 5);
        assert_eq!(state.surrounding_anchor, 11);
    }

    #[test]
    fn test_content_purpose() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Email, ContentHint::NONE);

        let state = ime.get_state(1).unwrap();
        assert_eq!(state.content_purpose, ContentPurpose::Email);
    }

    #[test]
    fn test_multiple_clients() {
        let mut ime = ImeManager::new();

        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);
        ime.activate(2, ContentPurpose::Email, ContentHint::COMPLETION);

        assert!(ime.is_active(1));
        assert!(ime.is_active(2));
        assert_eq!(ime.focused_client(), Some(2));
    }

    #[test]
    fn test_remove_client() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);

        ime.remove_client(1);
        assert!(!ime.is_active(1));
        assert!(ime.get_state(1).is_none());
    }

    #[test]
    fn test_inactive_operations() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);
        ime.deactivate(1);

        let result = ime.set_preedit_string(1, "test".to_string(), 0, 4);
        assert!(result.is_err());
    }

    #[test]
    fn test_serial_increment() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);

        let initial_serial = ime.get_state(1).unwrap().serial;

        ime.set_preedit_string(1, "test".to_string(), 0, 4)
            .unwrap();
        let after_preedit = ime.get_state(1).unwrap().serial;

        assert!(after_preedit > initial_serial);
    }

    #[test]
    fn test_stats() {
        let mut ime = ImeManager::new();

        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);
        ime.set_preedit_string(1, "test".to_string(), 0, 4)
            .unwrap();
        ime.commit_string(1, "test".to_string()).unwrap();

        let stats = ime.stats();
        assert_eq!(stats.total_activations, 1);
        assert_eq!(stats.preedit_updates, 1);
        assert_eq!(stats.text_commits, 1);
        assert_eq!(stats.active_clients, 1);
    }

    #[test]
    fn test_change_reason() {
        let mut ime = ImeManager::new();
        ime.activate(1, ContentPurpose::Normal, ContentHint::NONE);

        ime.set_change_reason(1, ChangeReason::InputMethod)
            .unwrap();

        let state = ime.get_state(1).unwrap();
        assert_eq!(state.change_reason, ChangeReason::InputMethod);
    }
}
