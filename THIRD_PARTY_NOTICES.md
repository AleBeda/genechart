# Third-party notices

genechart itself is distributed under the MIT License (see `LICENSE`). Its Rust
dependencies are fetched and built by Cargo from crates.io; their license texts are
available in the source of each crate and via `cargo about` / `cargo deny`. The crate
licenses are permissive (MIT / Apache-2.0 / BSD / Unicode), compatible with MIT
redistribution.

## Bundled component: Lua 5.4 (only when built with `--features lua`)

The optional `lua` feature pulls in [`mlua`](https://crates.io/crates/mlua) with its
`vendored` Lua 5.4, which is **compiled from source and statically linked into the
genechart binary**. If you distribute a binary built with `--features lua`, you must
retain the Lua copyright notice below (Lua is MIT-licensed).

A binary built **without** the `lua` feature (the default) contains no Lua code and this
section does not apply.

```
Copyright © 1994–2024 Lua.org, PUC-Rio.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```
