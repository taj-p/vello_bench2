## WebGL Benchmark

**[Live Site](https://laurenz-canva.github.io/vello_bench2/)**

A browser-based benchmark tool for Vello Hybrid's WebGL2 renderer. Two modes:

- **Interactive** -- tweak parameters in real-time, observe FPS.
- **Benchmark** -- automated suite with warmup calibration, vsync-independent timing, and comparison reports.

## Running

### Quick (single build)

Run with SIMD enabled (recommended):

```
RUSTFLAGS=-Ctarget-feature=+simd128 cargo run -- --package vello_bench2 --release
```

Scalar (non-SIMD) build:

```
cargo run -- --package vello_bench2 --release
```

### Full local server (SIMD toggle)

Builds both SIMD and non-SIMD variants and serves them with a toggle button in the top bar:

```
./serve.sh
```

Then open http://localhost:8080. Requires `wasm-bindgen-cli` (`cargo install wasm-bindgen-cli --version 0.2.114`).

### A/B testing a local Vello branch

Build both the pinned upstream Vello (control) and your local checkout (treatment)
in a single command, then toggle between them in the browser:

```
./ab.sh ~/repos/vello
```

This:

1. Builds the **control** variant using the git revision pinned in `Cargo.toml`.
2. Temporarily patches `Cargo.toml` to point at your local Vello checkout and builds
   the **treatment** variant. (`Cargo.toml` is restored automatically, even on error.)
3. Serves both at http://localhost:8080 with a **CONTROL / TREATMENT** toggle button
   in the top bar.

Run benchmarks on one variant, click the toggle, run them again -- deltas against
the other variant appear automatically.

Use `--rev` to override the control's git revision without editing `Cargo.toml`:

```
./ab.sh ~/repos/vello --rev abc123def
```

Use `--global` to bind to `0.0.0.0` (useful for testing on a tablet over the local network):

```
./ab.sh ~/repos/vello --rev abc123def --global
```

A/B mode always builds with SIMD enabled (the SIMD toggle is hidden). Use
`serve.sh` if you need to compare SIMD vs non-SIMD.