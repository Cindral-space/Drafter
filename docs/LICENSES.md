# Third-party implementation notices

## OpenSKP record layout logic

The independent Rust SketchUp reader in `crates/cindral-skp/src/container.rs`,
`modern.rs` and `legacy.rs` derives container/record decoding logic from the
MIT-licensed OpenSKP project by Ahsan Mehmood. Source studied outside this
repository: `https://github.com/iamahsanmehmood/openskp`, revision
`525093be16d52f64e0f0ccb2e56cc91648eabeb9`. The reference is not vendored.
The dev comparison script imports that external checkout's Python reader.

No reference model is redistributed. OpenSKP's repository-wide MIT notice was
found; model-specific licensing for its mixed real-world fixture corpus was
not established. Those models are neither copied nor committed here. Tests
created here author their own bytes under this repository's license. User
models stay in ignored `tests/fixtures/skp-local/`.

MIT License

Copyright (c) 2024-2026 Ahsan Mehmood

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
