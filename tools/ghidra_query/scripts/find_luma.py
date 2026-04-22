"""find-luma · scan for RGB->Gray weight triplets (0.299/0.587/0.114 etc)"""
import struct


def run(program, args):
    memory = program.getMemory()
    # Candidate weight patterns (R, G, B) triplets known to appear in luma code
    candidates = [
        ("BT.601 Y' ", (0.299, 0.587, 0.114)),
        ("BT.709    ", (0.2126, 0.7152, 0.0722)),
        ("SMPTE 240M", (0.212, 0.701, 0.087)),
        ("HDTV Y     ", (0.2627, 0.6780, 0.0593)),
        ("1/3 avg    ", (0.3333, 0.3333, 0.3333)),
        ("sRGB D50 Y", (0.2225, 0.7169, 0.0606)),  # sRGB->XYZ Y row (D50 adapted)
        ("sRGB D65 Y", (0.2126, 0.7152, 0.0722)),
        ("AdobeRGB Y", (0.2973, 0.6274, 0.0753)),
        ("Has 330Skel?", (0.333, 0.334, 0.333)),
    ]
    TOL = 0.002

    for block in memory.getBlocks():
        name = block.getName()
        if name not in (".rdata", ".data", ".text"):
            continue
        start = block.getStart()
        size = block.getSize()
        print(f"\nScanning {name}: 0x{start} size={size}")
        data = bytearray(size)
        for i in range(size):
            data[i] = memory.getByte(start.add(i)) & 0xFF

        # Scan every 4-byte aligned position
        for i in range(0, size - 12, 4):
            try:
                triple = struct.unpack("<fff", bytes(data[i:i+12]))
            except Exception:
                continue
            for label, (r, g, b) in candidates:
                if (abs(triple[0] - r) < TOL and
                    abs(triple[1] - g) < TOL and
                    abs(triple[2] - b) < TOL):
                    addr = start.add(i)
                    print(f"  0x{addr} [{label}]: ({triple[0]:.6f}, {triple[1]:.6f}, {triple[2]:.6f})")
                # Also try reversed (B, G, R)
                if (abs(triple[0] - b) < TOL and
                    abs(triple[1] - g) < TOL and
                    abs(triple[2] - r) < TOL):
                    addr = start.add(i)
                    print(f"  0x{addr} [{label} REV]: ({triple[0]:.6f}, {triple[1]:.6f}, {triple[2]:.6f})")

        # Also scan for doubles (8-byte aligned)
        for i in range(0, size - 24, 8):
            try:
                triple = struct.unpack("<ddd", bytes(data[i:i+24]))
            except Exception:
                continue
            for label, (r, g, b) in candidates:
                if (abs(triple[0] - r) < TOL and
                    abs(triple[1] - g) < TOL and
                    abs(triple[2] - b) < TOL):
                    addr = start.add(i)
                    print(f"  0x{addr} [{label} f64]: ({triple[0]:.6f}, {triple[1]:.6f}, {triple[2]:.6f})")
