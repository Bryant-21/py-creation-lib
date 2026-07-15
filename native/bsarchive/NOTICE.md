# Provenance Notes

`py_creation_lib/native/bsarchive/` is a project-owned native archive module. The build links
only the code present in this crate.

## Internal reference material

- `py_creation_lib/python/creation_lib/ba2/ba2_reader.py` and `py_creation_lib/python/creation_lib/ba2/bsa_reader.py` used as in-tree Python
  oracles for integration checks.

## Port tracking policy

If any function is ported directly from a reference file, annotate the Rust
source with a citation comment and add a short entry here naming the target and
 the internal reference path used.
