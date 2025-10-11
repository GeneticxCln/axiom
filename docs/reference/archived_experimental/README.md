# Archived Experimental Backends

## Overview

This directory contains archived experimental backend implementations from the Axiom compositor's development history. These files were moved here during the backend consolidation on **2025-10-11** as part of focusing the project on a single, production-quality Smithay backend.

## ⚠️ Important Notice

**This code is NOT maintained and is provided for historical/educational purposes only.**

- ❌ **Not part of active codebase** - These files are not compiled or tested
- ❌ **May not build** - Dependencies and APIs have evolved since these were written
- ❌ **Not recommended for production** - Use the production backend in `src/smithay/` instead
- ✅ **Historical reference only** - Useful for understanding development approaches and design decisions

## What's Here

### Directory Structure

```
archived_experimental/
└── experimental/          # Original src/experimental/ contents
    ├── mod.rs            # Deprecated module marker
    └── smithay/          # Various Smithay backend experiments
        ├── axiom_real_compositor.rs
        ├── minimal_server.rs
        ├── mod.rs
        ├── multi_output.rs
        ├── real_input.rs
        ├── real_smithay.rs
        ├── real_window.rs
        ├── smithay_backend.rs
        ├── smithay_backend_minimal.rs
        ├── smithay_backend_phase6.rs
        ├── smithay_backend_phase6_2.rs
        ├── smithay_backend_production.rs
        ├── smithay_backend_real.rs
        ├── smithay_backend_real_minimal.rs
        ├── smithay_backend_simple.rs
        ├── smithay_backend_working.rs
        ├── smithay_enhanced.rs
        └── wayland_protocols.rs
```

### Statistics

- **Total Files**: 19 Rust source files
- **Lines of Code**: ~6,782 lines
- **Size**: 280 KB
- **Archive Date**: 2025-10-11

## Purpose of These Implementations

During Axiom's development, these experimental backends served various purposes:

### 1. **Learning and Prototyping**
- Experimenting with different Smithay integration approaches
- Testing protocol implementations before committing to the production design
- Understanding Wayland compositor architecture through iteration

### 2. **Phased Development**
- Phase 6 variations (`phase6.rs`, `phase6_2.rs`) represent different stages of compositor development
- Each phase added more functionality incrementally
- Allowed testing partial implementations without breaking the main codebase

### 3. **Feature Experimentation**
- Testing multi-output support patterns
- Exploring different input handling architectures
- Prototyping window management approaches

### 4. **Minimal Implementations**
- `minimal_server.rs` - Bare-bones Wayland server for testing protocol basics
- `smithay_backend_minimal.rs` - Stripped-down compositor for debugging
- Useful for isolating and understanding specific protocol behaviors

## Why Were They Archived?

The decision to archive these implementations was made because:

1. **Production Backend Maturity**: The main Smithay backend in `src/smithay/` is now fully functional and production-ready
2. **Maintenance Overhead**: Multiple backends increased build times and complexity
3. **Code Clarity**: Having one canonical implementation makes the codebase easier to understand
4. **Focus**: Development effort should be concentrated on the production backend

See `../BACKEND_CONSOLIDATION_COMPLETE.md` for full details on the consolidation process.

## Production Backend

**Current Location**: `src/smithay/`

The production backend includes all the functionality from these experiments, refined and tested:

- ✅ Full XDG Shell protocol support
- ✅ WLR Layer Shell for desktop components
- ✅ Multi-output support
- ✅ SHM and DMA-BUF buffer handling
- ✅ Security module integration
- ✅ libinput backend for real hardware
- ✅ Decoration management (CSD/SSD)
- ✅ Subsurface support
- ✅ Clipboard and selection management
- ✅ Presentation time feedback
- ✅ Damage tracking and optimized rendering

## Using This Archive

### Acceptable Use Cases

✅ **Learning Resource**: Study different approaches to Smithay integration  
✅ **Historical Reference**: Understand why certain design decisions were made  
✅ **Code Examples**: See how specific Wayland protocols were handled  
✅ **Research**: Compare implementations for academic or educational purposes  

### Inappropriate Use Cases

❌ **Production Code**: Do not copy these implementations into production systems  
❌ **New Features**: Do not base new features on this code; use the production backend  
❌ **Bug Fixes**: If you find issues here, they're likely already fixed in production  
❌ **Dependencies**: Do not create dependencies on this archived code  

## If You Need to Reference This Code

1. **Check the production backend first** - Most functionality has been incorporated there with improvements
2. **Understand the context** - Read the phase documentation to understand what problem each implementation solved
3. **Verify against current APIs** - Wayland and Smithay APIs have evolved; this code reflects older versions
4. **Ask questions** - If you're unsure about something, ask the development team rather than assuming this code is current

## Related Documentation

- **Backend Consolidation**: `../BACKEND_CONSOLIDATION_COMPLETE.md`
- **Production Backend Guide**: `../../phases/PHASE_6_4_SMITHAY_INTEGRATION_COMPLETE.md`
- **Smithay Integration Status**: `../../phases/PHASE_5_SMITHAY_INTEGRATION_STATUS.md`
- **Architecture Overview**: `../../README.md`

## Development Phases Context

These files correspond to various development phases:

- **Phase 5**: Initial Smithay integration attempts
- **Phase 6**: Real compositor functionality
- **Phase 6.1**: Window management refinements
- **Phase 6.2**: Multi-window support
- **Phase 6.3**: Advanced protocol features
- **Phase 6.4**: Production readiness and optimization

Each numbered backend file (`phase6.rs`, `phase6_2.rs`, etc.) represents a snapshot from that development stage.

## Conclusion

This archive preserves the evolutionary history of Axiom's Wayland compositor backend. While this code is no longer part of the active codebase, it documents the journey from early experiments to the robust production implementation. Use it to learn, but always prefer the current production backend for any new development.

For questions about this archive or the consolidation process, see the main documentation or contact the development team.

---

**Archived**: 2025-10-11  
**Original Location**: `src/experimental/`  
**Production Backend**: `src/smithay/`  
**Status**: Historical reference only - not maintained
