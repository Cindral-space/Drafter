//! Camera math and persistent GPU resources for Cindral Drafter's viewport.

pub mod camera;
pub mod capture;
pub mod grid;
pub mod materials;
pub mod mesh;
pub mod scene;
pub mod texture;

pub use materials::{MaterialParams, MaterialTable, MATERIAL_TEXELS, MAX_MATERIALS};
pub use mesh::{
    EdgeVertex, FaceStyle, FaceVertex, GeometryBuffers, MeshCache, MeshUpdate, ModelCallback,
    ModelRenderer, FACE_COLOR,
};
pub use scene::{
    DefinitionUpdate, InstanceBatch, InstanceData, SceneCallback, SceneRenderer, SceneUpdate,
};
pub use texture::{
    TextureError, TexturePixels, DEFAULT_WHITE_LAYER, MAX_TEXTURE_LAYERS, PREVIEW_SIZE,
};

mod shadow;
