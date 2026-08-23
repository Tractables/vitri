# Third-party components

`vitri` is Apache-2.0 (`LICENSE`). This file records every third-party component
that ships inside the crate or is linked into a build of it, and the licence each
is used under. For the BSD-2-Clause, BSL-1.0 and MIT components below,
reproducing the notice is a **licence condition**, not a courtesy — the required
texts are at the end of this file.

Everything listed here is permissive, with one file-level exception: Eigen
(MPL-2.0, bundled inside SBVA, see § The Arjun stack), which carries no relinking
obligation.

## Rust dependencies

`num-bigint`, `num-rational`, `num-traits`, `rand`, `rustc-hash`, `libc`,
`serde`, `serde_json`, and the build-dependency `cc`. Each is dual-licensed
`MIT OR Apache-2.0` and is used here under Apache-2.0.

## FlowCutter tree decomposition — vendored C++, statically linked

The FlowCutter tree-decomposition backend is compiled from source by `build.rs`
and linked in. Third-party sources live in `vendor/treedecomp/upstream/` of the
vitri repository; paths in the table below are relative to it.

| Files | Upstream | Licence | Copyright |
|---|---|---|---|
| `flow-cutter-pace17/**`, `IFlowCutter.{cpp,hpp}` | [FlowCutter, PACE 2017](https://github.com/kit-algo/flow-cutter-pace17) | BSD-2-Clause | © 2016 Ben Strasser |
| `graph.{cpp,hpp}`, `bitset.hpp`, `utils.hpp` | sharpSAT-TD | MIT | © 2021 Tuukka Korhonen and Matti Järvisalo |
| `TreeDecomposition.{cpp,hpp}` | treedecomp | MIT | © 2023 Kenji Hashimoto |
| `treedecomp_defs.hpp` | treedecomp | MIT | © 2023 Authors of treedecomp |
| `time_mem.hpp` | MiniSat / CryptoMiniSat | MIT | © 2003–2006 Niklas Eén, Niklas Sörensson; © 2009–2020 the CryptoMiniSat authors |

One directory up, outside `upstream/`, sit the three files that are vitri's own
and carry vitri's licence: `ffi.cpp` / `ffi.h`, the C ABI shim over the above,
and `heap_selftest.cpp`, which drives upstream's k-way id-heap from a unit test.
The split is the boundary itself — everything under `upstream/` is somebody
else's source, and nothing else is.

**These files are modified.** The BSD-2-Clause and MIT licences above permit
modification and require the notice be kept, which it is; this states the changes
for a reader who expects upstream source. Most are marked in place with a
`// vitri:` comment naming the reason:

- missing `<cstdint>` / `<cstddef>` includes added, for types the upstream files
  use but only received transitively on the standard libraries they were written
  against;
- two memory-safety guards, in `IFlowCutter.cpp` and
  `flow-cutter-pace17/src/heap.hpp`: a budget on the bag-adjacency arc count and
  64-bit child-index arithmetic in the k-ary heap. Both convert an
  out-of-memory or integer-overflow crash on a pathological graph into a clean
  error the caller can handle;
- a null check on the `sspp::Bitset` allocation, in `bitset.hpp`, for the same
  reason;
- a density gate on FlowCutter's min-shortcut ordering heuristic, in
  `IFlowCutter.cpp`, so a clique-dominated graph does not spend the whole
  construction budget in one heuristic;
Upstream's heap arithmetic is also exercised from a unit test, through
`heap_selftest.cpp`; that file is vitri's own and adds nothing to the upstream
sources.

Diffing `vendor/treedecomp/upstream/` against the upstream projects named in the
table shows each of the modifications above.

## CaDiCaL — vendored C++, built by `build.rs`

The CaDiCaL SAT solver, reached through vitri's own C shim
(`vendor/arjun/cadical_shim.cpp`, which carries vitri's licence). The solver
itself is the copy vendored at `vendor/arjun/upstream/cadical` — the same one the
Arjun stack builds, so there is exactly one CaDiCaL in the process and no
separate crates.io dependency.

- CaDiCaL — MIT, © Armin Biere (meelgroup's fork; see `PROVENANCE.md`)

## The Arjun stack — vendored C++, built by `build.rs`

Sources for the whole Arjun preprocessing stack are vendored under
`vendor/arjun/upstream/` and built from source by `build.rs`, fully offline. See
`vendor/arjun/upstream/PROVENANCE.md` for the exact pinned commit of each tree.
The stack is MIT apart from the files in § Other licences in the Arjun stack
below:

- [Arjun](https://github.com/meelgroup/arjun), pinned release v2.7.2 — MIT, © Mate Soos.
  `src/arjun_c.{cpp,h}` and `src/arjun_c_priv.hpp` attribute copyright to
  "Authors of Arjun, see AUTHORS file"; this vendored subset carries no
  `AUTHORS` file, so that list is the one in the upstream repository above.
- [CryptoMiniSat](https://github.com/msoos/cryptominisat) — MIT, © Mate Soos
- the `sspp` / oracle solver bundled by CryptoMiniSat — MIT
- [cadiback](https://github.com/meelgroup/cadiback) — MIT
- PicoSAT, bundled inside CryptoMiniSat at
  `vendor/arjun/upstream/cryptominisat/src/mpicosat/` — MIT,
  © 2006–2014 Armin Biere, Johannes Kepler University
- SBVA (structured bounded variable addition) — MIT
- meelgroup's fork of CaDiCaL — MIT
- Eigen 3.4.0, bundled inside SBVA — MPL-2.0, with some files under
  BSD-3-Clause or LGPL 2.1. The BSD-3-Clause ones are Eigen's BLAS/MKL bindings
  — `Eigen/src/**/*_BLAS.h`, `Assign_MKL.h`, `MKL_support.h` — © 2011 Intel
  Corporation, carrying the "Neither the name of Intel Corporation" endorsement
  clause; their text ships as `sbva/eigen-3.4.0/COPYING.BSD`. The build compiles
  SBVA with `EIGEN_MPL2_ONLY`, which turns including an LGPL-licensed Eigen
  header into a compile error, so the MPL2-only property is enforced by the
  build, not by inspection. MPL-2.0 is file-level copyleft with no relinking
  obligation and is fine to redistribute alongside Apache-2.0.

### Other licences in the Arjun stack

Files under `vendor/arjun/upstream/` that are not MIT. All of them ship — the
package includes `vendor/**` whole — and two are compiled:

| Files | Licence | Copyright | Compiled |
|---|---|---|---|
| `cadical/contrib/craigtracer.{cpp,hpp}` | MIT OR Apache-2.0 | © 2013 Stefan Kupferschmid; © 2023 Florian Pollitt; © 2023 Tobias Faller | yes — `cadical/CMakeLists.txt` globs `contrib/*.cpp` into the library |
| `sbva/src/getdelim.h`, `sbva/src/getline.h` | BSD-2-Clause | © 2011 The NetBSD Foundation, Inc. | yes — `sbva.cpp` includes `getline.h` unconditionally |
| `{arjun,cryptominisat,sbva}/cmake/GetGitRevisionDescription.{cmake,cmake.in}` | BSL-1.0 | © Iowa State University 2009–2010 | no — CMake module and its template, vendored identically in all three trees |
| `cryptominisat/cmake/{Rust,Findcargo,Findrustc,Findrustdoc}.cmake` | zlib | © 2014 Pavel Sountsov | no — CMake modules |

Each of these keeps its own notice in the file, which is what its licence asks
of a source redistribution. The BSL-1.0 copies are the exception: they point at
an accompanying `LICENSE_1_0.txt` that upstream did not vendor, so the text is
reproduced at the end of this file.

CryptoMiniSat's licence propagates GPL when it is built **against Bliss**; the
pinned configuration does not build Bliss, and no Bliss, m4ri, louvain/community
or BreakID object appears in the linked archives.

**GMP and MPFR** (LGPLv3+; GMP additionally GPLv2+) are pulled in by that stack
and are linked **dynamically**, which is what keeps an Apache-2.0 distribution
clean under the LGPL.

**Arjun, CryptoMiniSat and cadiback are modified**, as the MIT licence permits:
they carry an added in-process wall-clock deadline, and Arjun's `CMakeLists.txt`
accepts an externally supplied `GIT_SHA1`. The changes are applied to the source
as vendored — there is no patch step —, and diffing against the pinned upstream
commits in
[`vendor/arjun/upstream/PROVENANCE.md`](vendor/arjun/upstream/PROVENANCE.md)
shows them.

`vendor/arjun/arjun_shim.{cpp,h}` is vitri's own C ABI shim and carries vitri's
licence.

No third-party test data ships with this crate: every test fixture is generated
in test code from a construction written here.

No GPL-licensed component is included in or linked by this crate.

---

## BSD-2-Clause — FlowCutter

```
Copyright (c) 2016, Ben Strasser
All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

Redistributions of source code must retain the above copyright notice, this list
of conditions and the following disclaimer.
Redistributions in binary form must reproduce the above copyright notice, this
list of conditions and the following disclaimer in the documentation and/or
other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## MIT

Applies to the MIT-licensed components above, each under its own copyright line
as listed:

```
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

## BSL-1.0 — `GetGitRevisionDescription`

Applies to the vendored copies of one upstream CMake module —
`cmake/GetGitRevisionDescription.cmake` and its `.cmake.in` template under each
of `vendor/arjun/upstream/{arjun,cryptominisat,sbva}/`, © Iowa State University
2009–2010. Each refers the reader to an accompanying `LICENSE_1_0.txt`;
upstream vendored none, so it is here.

```
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or
organization obtaining a copy of the software and accompanying
documentation covered by this license (the "Software") to use,
reproduce, display, distribute, execute, and transmit the
Software, and to prepare derivative works of the Software, and
to permit third-parties to whom the Software is furnished to do
so, all subject to the following:

The copyright notices in the Software and this entire statement,
including the above license grant, this restriction and the
following disclaimer, must be included in all copies of the
Software, in whole or in part, and all derivative works of the
Software, unless such copies or derivative works are solely in
the form of machine-executable object code generated by a source
language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES
OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, TITLE AND
NON-INFRINGEMENT. IN NO EVENT SHALL THE COPYRIGHT HOLDERS OR
ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE FOR ANY DAMAGES OR
OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE, ARISING
FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR
OTHER DEALINGS IN THE SOFTWARE.
```
