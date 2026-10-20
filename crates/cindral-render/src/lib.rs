//! Camera math and persistent GPU resources for Cindral Drafter's viewport.

pub mod camera;
pub mod grid;
pub mod mesh;
pub mod scene;

pub use mesh::{
    EdgeVertex, FaceStyle, FaceVertex, GeometryBuffers, MeshCache, MeshUpdate, ModelCallback,
    ModelRenderer, FACE_COLOR,
};
pub use scene::{
    DefinitionUpdate, InstanceBatch, InstanceData, SceneCallback, SceneRenderer, SceneUpdate,
};
