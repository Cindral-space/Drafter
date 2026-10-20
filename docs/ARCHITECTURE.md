# Architecture

## Status

M0 establishes compileable crate boundaries and cross-platform CI. Geometry and file APIs below are design intent, not implemented functionality. Public interfaces are frozen by the integrating engineer before dependent tasks are delegated.

## Crate boundaries

| Crate | Owns | Allowed application dependencies |
| --- | --- | --- |
| cindral-core | Topology, entities, transforms, command history | none |
| cindral-inference | Geometric queries and snap candidates | core |
| cindral-tools | Tool state machines and command creation | core, inference |
| cindral-render | Camera, GPU resources, picking and styles | core |
| cindral-io | Versioned documents and import/export | core |
| cindral-app | Window, egui controls, input routing | all above |

Rendering and UI dependencies never enter core. Tools emit commands; they do not mutate renderer buffers. IO deserializes into validated core documents.

## Planned core data model

A document owns stable typed entity IDs and ordered maps of vertices, undirected edges, planar faces, groups, component definitions/instances, tags, and materials. Positions use double precision. One shared tolerance constant governs geometric comparisons; derived predicates account for scale. A face has an outer vertex loop, zero or more inner loops, a plane and material. Edges carry soft/smooth flags. Adjacency is derived and validated after topology changes.

Groups own local geometry and transforms. Component instances refer to shared definitions; recursive references are rejected. Tags control visibility, independently of topology. Selections identify entities and their instance path.

Every document mutation runs through a transactional command: prepare and validate a candidate change, apply atomically, then record the inverse. Failed commands leave the document and history intact. New successful commands clear redo history. Render caches use document revision and dirty entity sets, with per-container GPU buffers and component instancing.

## Planned layers

Inference ranks endpoint, midpoint, intersection, edge, face and directional candidates using screen-space distances. Tools hold interaction state and interpret measurements in document units. Camera math is independent of window events. Picking shares camera transforms with rendering. `.cdraft` will be a versioned compact archive with explicit migrations and validation; exporters triangulate faces including holes.

## Validation

Every crate has a test target. CI runs formatting, Clippy with warnings denied, and workspace tests on Linux, macOS and Windows. Kernel work adds example-based and property tests for topology and command reversibility. Performance claims require measured representative scenes; no 500k-triangle performance result exists yet.
