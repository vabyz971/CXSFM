//! macOS backend: Metal present hook — not implemented.
//!
//! CarX Street on macOS presents through Metal (`CAMetalLayer` /
//! `MTLDrawable::present`). Intercepting it means swizzling an
//! Objective-C method (`presentDrawable:`) via the `objc` runtime —
//! a well-understood mechanism, but nobody on the team currently builds
//! or tests on macOS, so this stays an explicit stub rather than dead,
//! untested code. The dispatcher in `super` routes here and surfaces
//! this exact message in the framework log.
//!
//! Next step when macOS matters: `method_exchangeImplementations` on the
//! view's layer `display`/`present` selector, then call through to the
//! original IMP and invoke `super::on_frame()`.

use super::{RenderBackend, RenderError, RenderHook};

/// macOS Metal present hook (stub).
pub struct MetalHook;

impl RenderHook for MetalHook {
    fn backend(&self) -> RenderBackend {
        RenderBackend::Metal
    }

    fn install(&self) -> Result<(), RenderError> {
        Err(RenderError::Unsupported(
            "Metal present hook not implemented (needs objc method swizzling)".into(),
        ))
    }

    fn uninstall(&self) -> Result<(), RenderError> {
        Ok(())
    }
}
