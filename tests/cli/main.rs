//! Integration tests: CUJs 1-8, G4 staleness, and the read-only tree
//! snapshot invariant, exercised against a throwaway fixture repo.

mod audit;
mod common;
mod cuj;
mod dashboard;
mod force_parse;
mod global_usage;
mod impact;
mod incremental;
mod invalidation;
mod memory_basics;
mod memory_doctor;
mod memory_enhancements;
mod memory_import;
mod memory_parity;
mod memory_session;
mod p8_impact;
mod reuse;
mod reuse_deep;
mod reuse_phase1;
mod reuse_phase1_guards;
mod reuse_phase2;
mod reuse_phase3;
mod reuse_phase4;
mod reuse_phase5;
mod robustness;
mod security_guards;
mod stats;
mod symbols;
