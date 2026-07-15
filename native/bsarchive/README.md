# Overview

`bsarchive_native` is ModBox21's native Bethesda archive library. It is the
project-owned Rust backend for reading, extracting, and packing archive formats
used across Oblivion, Fallout 3/New Vegas, Skyrim, Fallout 4, and Starfield.

The crate serves two roles in this repo:

- a local Rust implementation for archive read/write behavior
- a Python extension module exposed as `bsarchive_native`

# Scope

The library supports the project's archive workflows:

- enumerate archive contents
- extract individual files or full archives
- inspect archive format/version metadata
- pack loose files into BSA/BA2 outputs for supported game variants

## Pack Archive Types

The Python `pack_archive()` binding accepts project archive type names. 
Project-specific parity work and known gaps are tracked in `AUDIT.md`.

# Validation

Validation lives in two places:

- Rust unit tests embedded in the archive modules
- Python integration tests under `tests/`, which compare native behavior
  against the in-tree Python archive readers

# Ownership

This directory is maintained as a project fork inside ModBox21. Upstream repo
branding has been removed so the crate can evolve as a first-party component of
the toolkit.
