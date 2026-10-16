//! Render modules:
//! `camera`: camera state and orbit/pan/zoom.
//! `context`: device, surface, frame orchestration.
//! `math`: vector/matrix helpers.
//! `pipelines`: render pipelines and camera uniforms.
//! `scene`: GPU resources and draw batches.
//! `viewport`: surface/WebView2 composition.
pub mod camera;
pub mod context;
pub mod math;
pub mod pipelines;
pub mod scene;
pub mod viewport;
