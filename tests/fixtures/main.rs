//! Spec-mandated fixture corpus (R1 fix): star import, getattr dispatch,
//! variable reassignment, `import x as y`, factory-nested constructor
//! (negative), and module-qualified constructor. Each asserts the exact
//! resolution outcome on a minimal fixture.

mod common;
mod misc;
mod python;
mod rust;
mod typescript;
