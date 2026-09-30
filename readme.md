# C-Road Editor — How to Run

C-Road is a C code editor that runs in your web browser. You write C in the
editor, press Run, and it compiles and executes with `gcc`. The whole thing
is one binary — no Python, no Node, no server setup.

This guide covers **Option 1**: build it once on your machine, then run it
by clicking the binary.

---

## What you need

**On the machine where you build it (once):**

- Rust and Cargo → https://rustup.rs
- `gcc` on your `PATH`

That's it. After the first build, the resulting binary is self-contained —
you can copy it anywhere and run it by clicking.

**On any machine where you run the binary:**

- `gcc` on your `PATH`

Nothing else. No Rust, no Cargo, no runtimes, no libraries.

---

## Step 1 — Install Rust (one time, only if you don't have it)

Open a terminal:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh