<!-- SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0 -->

# Third-party notices

Kortecx itself is distributed under the [Sustainable Use License](LICENSE.md). It also builds on
third-party components that remain under their own licenses. Those licenses govern that content,
and the notices below are reproduced as those licenses require.

This file covers components whose source is vendored into this repository or statically linked
into a distributed binary. Rust crate and npm package dependencies resolved at build time carry
their own licenses; `cargo deny check` enforces the allow-list in `deny.toml`, and
`bindings/*/package-lock.json` records the npm side.

---

## llama.cpp / ggml

Vendored as a git submodule at `crates/kx-llamacpp-sys/llama.cpp` and **statically linked** into
`kx` when it is built with the `inference` feature. The prebuilt release binaries are FFI-free and
do not include it.

- Project: https://github.com/ggml-org/llama.cpp
- License: MIT

```
MIT License

Copyright (c) 2023-2026 The ggml authors

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
```

The full text also ships with the submodule at
`crates/kx-llamacpp-sys/llama.cpp/LICENSE`.

---

## Models

Model weights are **not** distributed with Kortecx. You bring your own. Each model carries its own
license and terms from its publisher — for example Gemma is released under the Gemma Terms of Use.
Check the license of any model you serve.
