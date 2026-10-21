#!/usr/bin/env python3
"""Dev-only read-only comparison against an external OpenSKP Python checkout.

No SDK/application binaries or reference assets are invoked. Only models supplied
as arguments or in ignored tests/fixtures/skp-local are read. No fixtures copied.
"""
import argparse
import json
import math
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]


def reference(path):
    from openskp import SkpFile
    from openskp._core import multiply_matrices, transform_point
    reader = SkpFile.open(str(path))
    model = reader.parse()
    layer_names = reader._parsed.get("layer_id_to_name", {})
    definitions = list(model.definitions.values()) + [model.root]
    bounds = None
    identity = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.]
    pending = [(model.root, identity, frozenset())]
    count = 0
    hidden = {layer.name for layer in model.layers if layer.hidden}
    while pending:
        definition, matrix, ancestors = pending.pop()
        count += 1
        if count > 100_000:
            raise ValueError("reference traversal exceeds 100k occurrence budget")
        face_edges = set()
        visible_vertices = set()
        for face in definition.faces.values():
            referenced = {edge for loop in face.loops for edge, _ in loop}
            face_edges.update(referenced)
            if face.hidden or layer_names.get(face.layer) in hidden:
                continue
            for edge_id in referenced:
                edge = definition.edges.get(edge_id)
                if edge is not None:
                    visible_vertices.update((edge.v1_id, edge.v2_id))
        for edge_id, edge in definition.edges.items():
            if edge_id not in face_edges and layer_names.get(edge.layer) not in hidden:
                visible_vertices.update((edge.v1_id, edge.v2_id))
        for vertex_id in visible_vertices:
            vertex = definition.vertices.get(vertex_id)
            if vertex is None:
                continue
            point = [v * .0254 for v in transform_point((vertex.x, vertex.y, vertex.z), matrix)]
            if not all(math.isfinite(v) for v in point):
                continue
            if bounds is None:
                bounds = [point.copy(), point.copy()]
            else:
                bounds = [[min(a, b) for a, b in zip(bounds[0], point)],
                          [max(a, b) for a, b in zip(bounds[1], point)]]
        for instance in definition.instances:
            if instance.hidden or instance.layer in hidden:
                continue
            key = instance.ref_idx
            if key in ancestors or len(ancestors) >= 63:
                continue
            if key in model.definitions:
                pending.append((model.definitions[key], multiply_matrices(matrix, instance.matrix), ancestors | {key}))
    return {"version": model.version,
            "faces": sum(len(d.faces) for d in definitions),
            "instances": sum(len(d.instances) for d in definitions),
            "bounds": bounds}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*", type=pathlib.Path)
    parser.add_argument("--reference", type=pathlib.Path, default=ROOT.parent / "openskp-ref")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--no-build", action="store_true", help="Use an already-built release skp_inspect")
    args = parser.parse_args()
    if not args.reference.is_dir():
        parser.error(f"Missing reference checkout: {args.reference}")
    files = args.files or sorted((ROOT / "tests/fixtures/skp-local").glob("*.skp"))
    if not files:
        print("SKIP: no local .skp fixtures; provide files or populate tests/fixtures/skp-local/")
        return 0
    sys.path.insert(0, str(args.reference / "packages/python/src"))
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "-q", "-p", "cindral-io", "--example", "skp_inspect"], cwd=ROOT, check=True, timeout=600)
    mismatches = 0
    for path in files:
        try:
            expected = reference(path.resolve())
            process = subprocess.run([str(ROOT / "target/release/examples/skp_inspect"), str(path.resolve())],
                                     capture_output=True, text=True, timeout=args.timeout)
            if process.returncode:
                raise ValueError(process.stderr.strip()[-1500:])
            actual = json.loads(process.stdout)
            differences = [f"{key}: OpenSKP={expected[key]} Rust={actual[key]}" for key in ("faces", "instances") if expected[key] != actual[key]]
            a, b = expected["bounds"], actual["bounds"]
            equal_bounds = a == b if a is None or b is None else all(math.isclose(x, y, rel_tol=1e-8, abs_tol=1e-7) for p, q in zip(a, b) for x, y in zip(p, q))
            if not equal_bounds:
                differences.append(f"bounds (meters): OpenSKP={a} Rust={b}")
            if differences:
                mismatches += 1
                print(f"MISMATCH {path.name} ({expected['version']}): " + "; ".join(differences))
            else:
                print(f"PASS {path.name} ({expected['version']}): faces={actual['faces']} instances={actual['instances']} bounds={b}")
            if actual["skipped"]:
                print(f"  Rust skipped {actual['skipped']} record(s): {actual['warnings'][:3]}")
        except Exception as error:
            mismatches += 1
            print(f"ERROR {path.name}: {error}")
    return 1 if mismatches else 0


if __name__ == "__main__":
    raise SystemExit(main())
