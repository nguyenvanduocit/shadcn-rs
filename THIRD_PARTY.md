# Third-party notices

The code in this repository is MIT (`LICENSE`). It ships, or follows, the work below under its own licence.

| What | Where | Licence | Its text |
|---|---|---|---|
| Inter 4.001 | `crates/gui/assets/Inter.ttf` | SIL Open Font License 1.1 | `crates/gui/assets/LICENSE-Inter.txt` |
| Geist Mono 1.900 | `crates/gui/assets/GeistMono.ttf` | SIL Open Font License 1.1 | `crates/gui/assets/LICENSE-GeistMono.txt` |
| Lucide icons (lucide-static 1.77.0; some derived from Feather) | `crates/gui/assets/icons/` | ISC, and MIT for the Feather icons | `crates/gui/assets/icons/LICENSE` |
| shadcn/ui: the components' design, styles and demo content | `crates/shadcn` | MIT | below |
| shadcn-vue: the components and demos `crates/shadcn` ports and the verdict compares against | `crates/shadcn` | MIT | below |
| Geist (variable, Latin subset) | `site/public/fonts/Geist.woff2` | SIL Open Font License 1.1 | `site/public/fonts/LICENSE-Geist.txt` |
| shadcn-vue's home page: the landing page's layout and styles | `site/public/` | MIT | below |
| GPUI (Zed): the rounded-box signed distance and the blurred rounded box (Raph Levien's approximation) its shaders draw quads and shadows with | `crates/gui/src/gpu/shaders.wgsl` | Apache-2.0 | `crates/gui/src/gpu/LICENSE-GPUI` |
| HarfBuzz hb-gpu (Behdad Esfahbod): its Slug fragment shader, ported to the curve text shader | `crates/gui/src/gpu/slug.wgsl` | HarfBuzz's "Old MIT" | below, and at the top of the file |

The Rust crates this workspace depends on carry their own licences; `cargo deny check licenses` (`deny.toml`) lists the
ones allowed.

## shadcn/ui

MIT License

Copyright (c) 2023 shadcn

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

## shadcn-vue

MIT License

Copyright (c) 2023 unovue

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

## HarfBuzz hb-gpu

Copyright (C) 2026  Behdad Esfahbod

This is part of HarfBuzz, a text shaping library.

Permission is hereby granted, without written agreement and without
license or royalty fees, to use, copy, modify, and distribute this
software and its documentation for any purpose, provided that the
above copyright notice and the following two paragraphs appear in
all copies of this software.

IN NO EVENT SHALL THE COPYRIGHT HOLDER BE LIABLE TO ANY PARTY FOR
DIRECT, INDIRECT, SPECIAL, INCIDENTAL, OR CONSEQUENTIAL DAMAGES
ARISING OUT OF THE USE OF THIS SOFTWARE AND ITS DOCUMENTATION, EVEN
IF THE COPYRIGHT HOLDER HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH
DAMAGE.

THE COPYRIGHT HOLDER SPECIFICALLY DISCLAIMS ANY WARRANTIES, INCLUDING,
BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS FOR A PARTICULAR PURPOSE.  THE SOFTWARE PROVIDED HEREUNDER IS
ON AN "AS IS" BASIS, AND THE COPYRIGHT HOLDER HAS NO OBLIGATION TO
PROVIDE MAINTENANCE, SUPPORT, UPDATES, ENHANCEMENTS, OR MODIFICATIONS.
