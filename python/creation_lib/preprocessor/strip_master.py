"""Strip a master reference from an ESP/ESM file header (raw binary edit).

Usage: python strip_master.py <esp_path> <master_to_remove> [output_path]

This removes the MAST+DATA subrecord pair from the TES4 header and updates
the TES4 record size. It does NOT remap FormIDs — any records referencing
the removed master will have stale master indices.

This is an experimental tool for testing purposes only.
"""

import sys
import struct
from pathlib import Path


def find_subrecords(header_data: bytes) -> list[tuple[str, int, int, bytes]]:
    """Parse TES4 subrecords. Returns list of (type, offset, total_size, data)."""
    subrecords = []
    pos = 0
    while pos < len(header_data):
        if pos + 6 > len(header_data):
            break
        sr_type = header_data[pos:pos+4].decode('ascii')
        sr_data_size = struct.unpack_from('<H', header_data, pos + 4)[0]
        sr_data = header_data[pos+6:pos+6+sr_data_size]
        total_size = 6 + sr_data_size
        subrecords.append((sr_type, pos, total_size, sr_data))
        pos += total_size
    return subrecords


def strip_master(esp_path: str, master_name: str, output_path: str = None):
    data = bytearray(Path(esp_path).read_bytes())

    # TES4 record header: type(4) + data_size(4) + flags(4) + formid(4) + vc(4) + form_version(2) + vc2(2) = 24 bytes
    rec_type = data[0:4].decode('ascii')
    if rec_type != 'TES4':
        print(f"ERROR: Not an ESP/ESM — first record is '{rec_type}', expected 'TES4'")
        return False

    rec_data_size = struct.unpack_from('<I', data, 4)[0]
    header_payload = bytes(data[24:24+rec_data_size])

    print(f"TES4 header: {rec_data_size} bytes of subrecord data")

    subrecords = find_subrecords(header_payload)

    # Find all MAST entries
    print("\nCurrent masters:")
    mast_indices = []
    for i, (sr_type, offset, size, sr_data) in enumerate(subrecords):
        if sr_type == 'MAST':
            name = sr_data.rstrip(b'\x00').decode('utf-8')
            print(f"  [{len(mast_indices)}] {name} (offset {offset}, {size} bytes)")
            mast_indices.append(i)

    # Find the MAST we want to remove
    target_idx = None
    target_mast_idx = None
    for mi, si in enumerate(mast_indices):
        sr_type, offset, size, sr_data = subrecords[si]
        name = sr_data.rstrip(b'\x00').decode('utf-8')
        if name.lower() == master_name.lower():
            target_idx = si
            target_mast_idx = mi
            break

    if target_idx is None:
        print(f"\nERROR: Master '{master_name}' not found in TES4 header")
        return False

    print(f"\nRemoving master [{target_mast_idx}]: {master_name}")

    # Each MAST is followed by a DATA subrecord (8 bytes of file size)
    # Verify the next subrecord is DATA
    data_idx = target_idx + 1
    if data_idx < len(subrecords) and subrecords[data_idx][0] == 'DATA':
        pass
    else:
        print(f"WARNING: Expected DATA subrecord after MAST, got {subrecords[data_idx][0] if data_idx < len(subrecords) else 'EOF'}")
        data_idx = None

    # Calculate bytes to remove
    mast_sr = subrecords[target_idx]
    bytes_to_remove = mast_sr[2]  # MAST subrecord total size
    remove_start = 24 + mast_sr[1]  # absolute offset in file

    if data_idx is not None:
        data_sr = subrecords[data_idx]
        bytes_to_remove += data_sr[2]  # DATA subrecord total size

    remove_end = remove_start + bytes_to_remove

    print(f"  Removing {bytes_to_remove} bytes at offset {remove_start}-{remove_end}")
    print(f"  Old TES4 data size: {rec_data_size}")

    # Remove the bytes
    del data[remove_start:remove_end]

    # Update TES4 record data size
    new_size = rec_data_size - bytes_to_remove
    struct.pack_into('<I', data, 4, new_size)
    print(f"  New TES4 data size: {new_size}")

    # Show what FormIDs in the file still reference master index target_mast_idx
    # In the binary, FormIDs use the master index as the highest byte(s)
    print(f"\n  WARNING: FormIDs referencing master index {target_mast_idx} are now orphaned!")
    print(f"  The game may resolve them incorrectly or as NULL.")

    # Save
    out = output_path or esp_path
    Path(out).write_bytes(bytes(data))
    print(f"\nSaved to: {out}")
    return True


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)

    esp = sys.argv[1]
    master = sys.argv[2]
    output = sys.argv[3] if len(sys.argv) > 3 else None

    if not strip_master(esp, master, output):
        sys.exit(1)
