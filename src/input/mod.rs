//! Input handling and key bindings
//! Manages keyboard, mouse, and gesture input

use anyhow::Result;
use crate::config::{InputConfig, BindingsConfig};

pub struct InputManager {
    input_config: InputConfig,
    bindings_config: BindingsConfig,
}

impl InputManager {
    pub fn new(input_config: &InputConfig, bindings_config: &BindingsConfig) -> Result<Self> {
        Ok(Self {
            input_config: input_config.clone(),
            bindings_config: bindings_config.clone(),
        })
    }
    
    pub fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
