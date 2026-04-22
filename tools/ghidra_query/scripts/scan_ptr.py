"""scan-ptr · find all occurrences of a 4-byte little-endian pointer in .rdata/.data"""
import struct


def run(program, args):
    target = int(args[0], 16)
    target_bytes = struct.pack("<I", target)
    memory = program.getMemory()
    for block in memory.getBlocks():
        name = block.getName()
        if name not in (".rdata", ".data"):
            continue
        start = block.getStart()
        size = block.getSize()
        data = bytearray(size)
        for i in range(size):
            data[i] = memory.getByte(start.add(i)) & 0xFF
        # search every 4-aligned position
        for i in range(0, size - 4, 4):
            if bytes(data[i:i+4]) == target_bytes:
                addr = start.add(i)
                print(f"  0x{addr} in {name}")
