// Native rendering pipeline. Replaces the in-page WebGL 2 renderer: broken
// only at the boundary where the integrator wires it into the app lifecycle.
// Module layout is pinned; ownership is discovered via the shared dispatch
// contract in this module's header documentation. Do not add modules here.
pub mod camera;
pub mod context;
pub mod math;
pub mod pipelines;
pub mod scene;
pub mod viewport;
