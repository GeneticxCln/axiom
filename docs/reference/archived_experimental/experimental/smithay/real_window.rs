//! Real window management using Smithay 0.3 wayland compositor + xdg shell

#[allow(unused_imports)]
use smithay::wayland::{
    compositor::{self},
    shell::xdg::{self, ToplevelSurface},
};

pub struct RealWindowManager;

impl RealWindowManager {
    pub fn new() -> Self { Self }
}

