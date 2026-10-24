// Only the debug `--demo` drives the island until app.rs wires it in (Task 10).
#![allow(dead_code)]

#[cfg(debug_assertions)]
pub mod demo;
pub mod geometry;
pub mod render;
pub mod window;
