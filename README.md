# assets

Supporting files for the documentation on `main`, kept off `main` so its
history stays text-only. Pages link here by raw URL
(`https://raw.githubusercontent.com/Tractables/vitri/assets/<path>`).

This branch has no shared history with `main` and is replaced wholesale when
files are regenerated; do not base work on it.

- `logo/` — the project logo for the READMEs, browser app and API documentation.
- `showcase/` — pictures for `docs/showcase.md`.

## Logo assets

`logo/vitri-logo.svg` is copied from `docs/logo.svg` on `main`, the editable
source. The PNG files are renders of that SVG; existing image URLs continue
to serve the current design. Regenerate them from a main checkout with
CairoSVG and Pillow installed:

```sh
python3 .github/scripts/render-logo.py /path/to/assets-checkout
```
