"""read-const · 读取指定地址的全局常量（float/double/int）

常用于解密反编译里的 _DAT_xxx 常量。
"""
import struct


def run(program, args):
    if not args:
        print("用法: ./run.sh read-const 0xADDR [0xADDR2 ...]")
        print("        ./run.sh read-const 0x70734060  # 单地址")
        return
    memory = program.getMemory()
    af = program.getAddressFactory()

    for target in args:
        addr_str = target[2:] if target.startswith("0x") else target
        addr = af.getAddress(addr_str)
        if addr is None:
            print(f"0x{addr_str}: 无效地址")
            continue

        # Try reading 8 bytes as double, 4 bytes as float, and as int
        try:
            b = bytearray(8)
            for i in range(8):
                b[i] = memory.getByte(addr.add(i)) & 0xFF
            d = struct.unpack("<d", bytes(b))[0]
            f4 = struct.unpack("<f", bytes(b[:4]))[0]
            i4 = struct.unpack("<i", bytes(b[:4]))[0]
            u4 = struct.unpack("<I", bytes(b[:4]))[0]

            hex_bytes = " ".join(f"{x:02x}" for x in b)
            print(f"0x{target}:")
            print(f"  raw:     {hex_bytes}")
            print(f"  double:  {d!r}")
            print(f"  float:   {f4!r}")
            print(f"  int32:   {i4}")
            print(f"  uint32:  0x{u4:08x} ({u4})")
        except Exception as e:
            print(f"0x{target}: 读取失败 {e}")
